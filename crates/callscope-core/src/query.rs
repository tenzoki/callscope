//! Pure graph query algorithms over a loaded [`Index`] — the logic behind
//! capabilities C1–C7. `callscope-mcp` (P7) wraps each of these in an MCP tool;
//! the query layer itself links no compiler internals and does no I/O, so the
//! whole module is testable on stable against small hand-built graphs.
//!
//! # Envelope composition
//!
//! Every query returns an [`Envelope<T>`] directly, so the answer carries its
//! own uncertainty rather than relying on the caller to reconstruct it:
//!
//! - **Over-approximation (Q2).** A walk that crosses a [`EdgeKind::Virtual`]
//!   (dyn-dispatch) edge cannot claim its callee set is exact. The envelope's
//!   `over_approximated` is set to a [`Reason::DynDispatch`] whose
//!   `implementor_count` is the number of distinct virtual callees the walk
//!   folded in. The schema's [`Edge`] does not carry the dispatched trait's
//!   identity in v1, so `trait_path` is the generic marker [`DYN_TRAIT_MARKER`];
//!   the indexer (P4) can enrich this later without changing this contract.
//! - **Boundary (v1 dependency edge).** A walk that crosses into a third-party
//!   crate sets `boundary_applies`. v1's schema has no dedicated boundary edge
//!   kind, so a boundary crossing is represented structurally: an edge whose
//!   target is either absent from the symbol table (the index filtered the
//!   third-party crate out) or present but `characteristics.foreign`. See
//!   [`Graph::is_boundary_target`].
//! - **Bounded output (Q4).** Every query that can return a large set accepts a
//!   `limit`, reports the full `total`, and sets `truncated` when the limit cut
//!   the answer short. The full set is always walked first (the graph is finite
//!   and cycle-safe via a visited set), so `total` is the true count, not a
//!   partial one.
//!
//! `stale` (Q6) is deliberately never set here — staleness is a property of the
//! index versus the current sources on disk, which the query layer cannot see.
//! `callscope-mcp` attaches it with [`Envelope::with_stale`] after running the
//! fingerprint check.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::envelope::{Envelope, Reason};
use crate::schema::{Edge, EdgeKind, Index, Symbol, SymbolId};

/// The `trait_path` recorded on a [`Reason::DynDispatch`] produced by a walk.
///
/// v1's [`Edge`] does not carry the dispatched trait's identity, so a query
/// cannot name the concrete trait. The over-approximation is still real and
/// reported; only the trait label is generic. When the indexer (P4) attaches
/// trait identity to virtual edges, this marker gives way to the real path.
pub const DYN_TRAIT_MARKER: &str = "<dyn dispatch>";

/// Which way a reachability walk runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Successors: what the start symbol can reach (things it calls).
    Forward,
    /// Predecessors: what can reach the start symbol (things that call it).
    Backward,
}

/// C2 payload: the direct callers and direct callees of one symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectCalls {
    /// Symbols with a direct edge *into* the queried symbol.
    pub callers: Vec<Symbol>,
    /// Symbols the queried symbol has a direct edge *to*.
    pub callees: Vec<Symbol>,
}

/// C4 payload: one enumerated call path, as the sequence of symbols visited from
/// the `from` end to the `to` end inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallPath {
    pub nodes: Vec<Symbol>,
}

/// C7 payload: the combined change-impact answer for one symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impact {
    /// Direct callers (1-hop predecessors) — who to check first.
    pub direct_callers: Vec<Symbol>,
    /// Tests that transitively reach the symbol — what to run.
    pub affected_tests: Vec<Symbol>,
}

/// Uncertainty accumulated while walking edges. Order-independent: two walks
/// that cross the same edges in any order produce the same flags.
#[derive(Debug, Default, Clone)]
struct WalkFlags {
    over_approximated: bool,
    /// Distinct targets reached across a virtual edge, so `implementor_count`
    /// counts implementors folded in rather than edges traversed.
    virtual_targets: BTreeSet<SymbolId>,
    boundary: bool,
}

impl WalkFlags {
    /// Fold another walk's flags into this one (used to combine sub-queries for
    /// C7 impact).
    fn merge(&mut self, other: &WalkFlags) {
        self.over_approximated |= other.over_approximated;
        self.boundary |= other.boundary;
        for id in &other.virtual_targets {
            self.virtual_targets.insert(*id);
        }
    }

    /// Stamp the accumulated uncertainty onto an envelope.
    fn apply_to<T>(&self, env: Envelope<T>) -> Envelope<T> {
        let env = env.with_boundary(self.boundary);
        if self.over_approximated {
            env.with_over_approximation(Reason::dyn_dispatch(
                DYN_TRAIT_MARKER,
                self.virtual_targets.len(),
            ))
        } else {
            env
        }
    }
}

/// A queryable view over a loaded [`Index`].
///
/// Built once via [`Graph::new`] — it precomputes the id lookup and the forward
/// and backward adjacency lists so repeated queries (as the MCP server issues)
/// don't re-scan the edge vector each time. Adjacency lists are sorted by the
/// neighbour's `fq_path`, which is what makes path enumeration and every result
/// deterministic across runs.
pub struct Graph<'a> {
    by_id: HashMap<SymbolId, &'a Symbol>,
    /// `from` -> outgoing edges, sorted by the `to` symbol's fq_path.
    forward: HashMap<SymbolId, Vec<&'a Edge>>,
    /// `to` -> incoming edges, sorted by the `from` symbol's fq_path.
    backward: HashMap<SymbolId, Vec<&'a Edge>>,
    symbols: &'a [Symbol],
}

impl<'a> Graph<'a> {
    /// Build the queryable view. O(V + E log E) for the adjacency sort.
    pub fn new(index: &'a Index) -> Self {
        let mut by_id: HashMap<SymbolId, &'a Symbol> = HashMap::with_capacity(index.symbols.len());
        for sym in &index.symbols {
            by_id.insert(sym.id, sym);
        }

        let mut forward: HashMap<SymbolId, Vec<&'a Edge>> = HashMap::new();
        let mut backward: HashMap<SymbolId, Vec<&'a Edge>> = HashMap::new();
        for edge in &index.edges {
            forward.entry(edge.from).or_default().push(edge);
            backward.entry(edge.to).or_default().push(edge);
        }

        // Sort adjacency by the neighbour's fq_path so traversal order — and
        // therefore path order and flag counting — is stable. Unknown targets
        // (boundary crossings) sort by their raw id, which is still stable.
        let key = |id: SymbolId, map: &HashMap<SymbolId, &'a Symbol>| -> (String, u64) {
            match map.get(&id) {
                Some(s) => (s.fq_path.clone(), id.0),
                None => (String::new(), id.0),
            }
        };
        for edges in forward.values_mut() {
            edges.sort_by(|a, b| key(a.to, &by_id).cmp(&key(b.to, &by_id)));
        }
        for edges in backward.values_mut() {
            edges.sort_by(|a, b| key(a.from, &by_id).cmp(&key(b.from, &by_id)));
        }

        Graph {
            by_id,
            forward,
            backward,
            symbols: &index.symbols,
        }
    }

    /// Whether crossing to `target` leaves the workspace (v1 boundary).
    ///
    /// Two structural signals, either of which marks a boundary: the target is
    /// absent from the symbol table (the index filtered its third-party crate
    /// out), or it is present but flagged `foreign` (an `extern`/FFI item). A
    /// future schema may add a dedicated boundary edge kind; until then this is
    /// how a walk knows it reached the workspace edge.
    fn is_boundary_target(&self, target: SymbolId) -> bool {
        match self.by_id.get(&target) {
            None => true,
            Some(sym) => sym.characteristics.foreign,
        }
    }

    /// Update `flags` for one traversed edge.
    fn note_edge(&self, edge: &Edge, flags: &mut WalkFlags) {
        if edge.kind == EdgeKind::Virtual {
            flags.over_approximated = true;
            flags.virtual_targets.insert(edge.to);
        }
        if self.is_boundary_target(edge.to) {
            flags.boundary = true;
        }
    }

    /// The neighbour id an edge points at, in the given direction.
    fn neighbour(edge: &Edge, dir: Direction) -> SymbolId {
        match dir {
            Direction::Forward => edge.to,
            Direction::Backward => edge.from,
        }
    }

    /// Outgoing/incoming edges of `id` in the given direction (already sorted).
    fn edges(&self, id: SymbolId, dir: Direction) -> &[&'a Edge] {
        let map = match dir {
            Direction::Forward => &self.forward,
            Direction::Backward => &self.backward,
        };
        map.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Clone the symbols named by `ids`, dropping ids that name no symbol
    /// (boundary targets), and sort by `fq_path` then `id` for determinism.
    fn materialize(&self, ids: impl IntoIterator<Item = SymbolId>) -> Vec<Symbol> {
        let mut out: Vec<Symbol> = ids
            .into_iter()
            .filter_map(|id| self.by_id.get(&id).map(|s| (*s).clone()))
            .collect();
        out.sort_by(|a, b| (a.fq_path.as_str(), a.id.0).cmp(&(b.fq_path.as_str(), b.id.0)));
        out
    }

    /// Full transitive reachable set from `start` in `dir`, excluding `start`
    /// itself (it appears only if a cycle leads back to it). Cycle-safe via a
    /// visited set; walks the whole finite component so callers get a true
    /// total. Returns the reachable ids and the uncertainty seen en route.
    fn reachable_set(&self, start: SymbolId, dir: Direction) -> (BTreeSet<SymbolId>, WalkFlags) {
        let mut result: BTreeSet<SymbolId> = BTreeSet::new();
        let mut enqueued: BTreeSet<SymbolId> = BTreeSet::new();
        let mut flags = WalkFlags::default();
        let mut queue: VecDeque<SymbolId> = VecDeque::new();

        enqueued.insert(start);
        queue.push_back(start);

        while let Some(node) = queue.pop_front() {
            for edge in self.edges(node, dir) {
                self.note_edge(edge, &mut flags);
                let next = Self::neighbour(edge, dir);
                // A boundary target is a leaf: it is not a workspace symbol we
                // descend into. Record the flag (done above) but do not walk it.
                if self.is_boundary_target(next) {
                    continue;
                }
                result.insert(next);
                if enqueued.insert(next) {
                    queue.push_back(next);
                }
            }
        }

        (result, flags)
    }

    // ── C1 ────────────────────────────────────────────────────────────────

    /// Resolve a name or fragment to the **candidate set** of matching symbols
    /// (C1). Never picks one (Q3): every symbol whose `fq_path` contains the
    /// query, case-insensitively, is returned, sorted by `fq_path`, bounded by
    /// `limit` with the full `total` reported. An empty query matches nothing.
    pub fn resolve(&self, query: &str, limit: usize) -> Envelope<Vec<Symbol>> {
        let needle = query.to_lowercase();
        let matches: Vec<SymbolId> = if needle.is_empty() {
            Vec::new()
        } else {
            self.symbols
                .iter()
                .filter(|s| s.fq_path.to_lowercase().contains(&needle))
                .map(|s| s.id)
                .collect()
        };
        let all = self.materialize(matches);
        let total = all.len();
        let (data, truncated) = truncate(all, limit);
        Envelope::exact(data)
            .with_total(total)
            .with_truncated(truncated)
    }

    // ── C2 ────────────────────────────────────────────────────────────────

    /// Direct callers and direct callees of `symbol` (C2). 1-hop each way.
    /// `total` is the combined count before truncation; each list is bounded to
    /// `limit` independently and `truncated` is set if either was cut. Virtual
    /// and boundary edges among the direct neighbours propagate their flags.
    pub fn direct_calls(&self, symbol: SymbolId, limit: usize) -> Envelope<DirectCalls> {
        let mut flags = WalkFlags::default();

        let mut caller_ids = Vec::new();
        for edge in self.edges(symbol, Direction::Backward) {
            self.note_edge(edge, &mut flags);
            caller_ids.push(edge.from);
        }
        let mut callee_ids = Vec::new();
        for edge in self.edges(symbol, Direction::Forward) {
            self.note_edge(edge, &mut flags);
            callee_ids.push(edge.to);
        }

        let callers_full = self.materialize(caller_ids);
        let callees_full = self.materialize(callee_ids);
        let total = callers_full.len() + callees_full.len();

        let (callers, callers_trunc) = truncate(callers_full, limit);
        let (callees, callees_trunc) = truncate(callees_full, limit);

        let env = Envelope::exact(DirectCalls { callers, callees })
            .with_total(total)
            .with_truncated(callers_trunc || callees_trunc);
        flags.apply_to(env)
    }

    // ── C3 ────────────────────────────────────────────────────────────────

    /// Transitive reachability from `symbol` in `dir` (C3). The full reachable
    /// set is walked (cycle-safe), so `total` is exact; the output is sorted and
    /// bounded to `limit` with `truncated` set when the limit cut it.
    pub fn reachability(
        &self,
        symbol: SymbolId,
        dir: Direction,
        limit: usize,
    ) -> Envelope<Vec<Symbol>> {
        let (set, flags) = self.reachable_set(symbol, dir);
        let all = self.materialize(set);
        let total = all.len();
        let (data, truncated) = truncate(all, limit);
        let env = Envelope::exact(data).with_total(total).with_truncated(truncated);
        flags.apply_to(env)
    }

    // ── C4 ────────────────────────────────────────────────────────────────

    /// Enumerate simple call paths from `from` to `to` (C4), each at most
    /// `max_depth` edges long, returning at most `max_paths` of them. Paths are
    /// simple (no repeated node), so cycles never blow the enumeration up.
    ///
    /// # `total` vs `truncated` — two distinct signals for C4
    ///
    /// These mean different things here than in C1/C3/C5/C6, and the difference
    /// is deliberate:
    ///
    /// - `total` is the number of paths **RETURNED** (i.e. after the `max_paths`
    ///   cap), **not** a true full path count. Unlike the reachable-set queries,
    ///   whose `total` is the exact pre-truncation size, a graph can hold an
    ///   exponential number of simple paths, so enumerating them all just to
    ///   count them can blow up. C4 therefore does not attempt a true total.
    /// - `truncated` is the "more may exist" signal. It is set when the
    ///   `max_paths` cap was **actually exceeded** — we enumerate one extra path
    ///   (`max_paths + 1`) purely to detect this, so a result with *exactly*
    ///   `max_paths` real paths reports `truncated = false` — **or** when a
    ///   branch was cut at `max_depth` before reaching `to` (a longer path may
    ///   have been missed).
    ///
    /// # Uncertainty flags cover every edge the enumeration touches
    ///
    /// Virtual (Q2 over-approximation) and boundary flags are accumulated during
    /// the DFS as each edge is traversed — not in a post-hoc pass over the
    /// returned paths. This matters when the cap drops a path: if a dropped path
    /// crossed a virtual or boundary edge, that uncertainty is still reported,
    /// rather than silently vanishing with the path. Residual limitation: once
    /// the enumeration hits its collect cap (`max_paths + 1`) it stops exploring,
    /// so edges reachable only along paths beyond that cap are not visited and
    /// their flags are not folded in. `truncated` marks such answers incomplete.
    pub fn call_paths(
        &self,
        from: SymbolId,
        to: SymbolId,
        max_depth: usize,
        max_paths: usize,
    ) -> Envelope<Vec<CallPath>> {
        let mut paths: Vec<Vec<SymbolId>> = Vec::new();
        let mut current: Vec<SymbolId> = vec![from];
        let mut on_path: BTreeSet<SymbolId> = BTreeSet::new();
        on_path.insert(from);
        let mut depth_cut = false;
        // Flags accumulate during traversal, so paths dropped by the cap below
        // still contribute their virtual/boundary uncertainty.
        let mut flags = WalkFlags::default();

        // Enumerate one path beyond the cap. That extra path is what lets us tell
        // "found exactly max_paths and nothing more" (not truncated) from "more
        // exist" (truncated) without enumerating a possibly-exponential full set.
        let collect_cap = max_paths.saturating_add(1);

        self.dfs_paths(
            from,
            to,
            max_depth,
            collect_cap,
            &mut current,
            &mut on_path,
            &mut paths,
            &mut depth_cut,
            &mut flags,
        );

        // We collected up to max_paths + 1; more than max_paths means the cap
        // genuinely cut the result short.
        let hit_cap = paths.len() > max_paths;

        // Deterministic order: sort paths by their fq_path sequence, then keep
        // only the first max_paths (dropping the probe path, and any excess).
        paths.sort_by(|a, b| self.path_key(a).cmp(&self.path_key(b)));
        paths.truncate(max_paths);

        let data: Vec<CallPath> = paths
            .into_iter()
            .map(|ids| CallPath {
                nodes: ids
                    .into_iter()
                    .filter_map(|id| self.by_id.get(&id).map(|s| (*s).clone()))
                    .collect(),
            })
            .collect();
        // `total` is the RETURNED count (see the doc comment); `truncated` is the
        // independent "more may exist" signal.
        let total = data.len();
        let env = Envelope::exact(data)
            .with_total(total)
            .with_truncated(hit_cap || depth_cut);
        flags.apply_to(env)
    }

    #[allow(clippy::too_many_arguments)]
    fn dfs_paths(
        &self,
        node: SymbolId,
        target: SymbolId,
        max_depth: usize,
        collect_cap: usize,
        current: &mut Vec<SymbolId>,
        on_path: &mut BTreeSet<SymbolId>,
        paths: &mut Vec<Vec<SymbolId>>,
        depth_cut: &mut bool,
        flags: &mut WalkFlags,
    ) {
        if paths.len() >= collect_cap {
            return;
        }
        if node == target && current.len() > 1 {
            paths.push(current.clone());
            return;
        }
        // `current.len() - 1` is the edge count so far; stop extending at the
        // depth bound and record that a branch was cut.
        if current.len().saturating_sub(1) >= max_depth {
            if self.edges(node, Direction::Forward).iter().any(|e| e.to != node) {
                *depth_cut = true;
            }
            return;
        }
        for edge in self.edges(node, Direction::Forward) {
            // Note every edge the enumeration traverses, so uncertainty is
            // captured independent of whether this path survives the cap.
            self.note_edge(edge, flags);
            let next = edge.to;
            if on_path.contains(&next) {
                continue; // keep the path simple; skip cycles
            }
            if self.by_id.get(&next).is_none() {
                continue; // boundary leaf, cannot continue a path through it
            }
            current.push(next);
            on_path.insert(next);
            self.dfs_paths(
                next, target, max_depth, collect_cap, current, on_path, paths, depth_cut, flags,
            );
            on_path.remove(&next);
            current.pop();
            if paths.len() >= collect_cap {
                return;
            }
        }
    }

    /// Lookup key for sorting a path: the fq_path of each node in order.
    fn path_key(&self, path: &[SymbolId]) -> Vec<String> {
        path.iter()
            .map(|id| {
                self.by_id
                    .get(id)
                    .map(|s| s.fq_path.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    // ── C5 ────────────────────────────────────────────────────────────────

    /// Tests that transitively reach `symbol` (C5) — backward reachability
    /// filtered to symbols with `characteristics.test`. This is the load-bearing
    /// capability: a test reaching the symbol only through a `dyn`-dispatch edge
    /// still appears here, and the envelope is over-approximated to say so (Q2).
    pub fn affected_tests(&self, symbol: SymbolId, limit: usize) -> Envelope<Vec<Symbol>> {
        let (set, flags) = self.reachable_set(symbol, Direction::Backward);
        let tests: Vec<SymbolId> = set
            .into_iter()
            .filter(|id| {
                self.by_id
                    .get(id)
                    .map(|s| s.characteristics.test)
                    .unwrap_or(false)
            })
            .collect();
        let all = self.materialize(tests);
        let total = all.len();
        let (data, truncated) = truncate(all, limit);
        let env = Envelope::exact(data).with_total(total).with_truncated(truncated);
        flags.apply_to(env)
    }

    // ── C6 ────────────────────────────────────────────────────────────────

    /// Unsafe-using symbols reachable forward from `symbol` (C6) — forward
    /// reachability filtered to `characteristics.uses_unsafe`.
    pub fn reachable_unsafe(&self, symbol: SymbolId, limit: usize) -> Envelope<Vec<Symbol>> {
        let (set, flags) = self.reachable_set(symbol, Direction::Forward);
        let unsafe_syms: Vec<SymbolId> = set
            .into_iter()
            .filter(|id| {
                self.by_id
                    .get(id)
                    .map(|s| s.characteristics.uses_unsafe)
                    .unwrap_or(false)
            })
            .collect();
        let all = self.materialize(unsafe_syms);
        let total = all.len();
        let (data, truncated) = truncate(all, limit);
        let env = Envelope::exact(data).with_total(total).with_truncated(truncated);
        flags.apply_to(env)
    }

    // ── C7 ────────────────────────────────────────────────────────────────

    /// Combined change impact for `symbol` (C7): its direct callers plus the
    /// tests that transitively reach it, in one answer. `total` is the combined
    /// count before truncation; each list is bounded to `limit`. Flags are the
    /// union of the 1-hop caller scan and the full backward test walk.
    pub fn impact(&self, symbol: SymbolId, limit: usize) -> Envelope<Impact> {
        let mut flags = WalkFlags::default();

        // Direct callers (1-hop backward).
        let mut caller_ids = Vec::new();
        for edge in self.edges(symbol, Direction::Backward) {
            self.note_edge(edge, &mut flags);
            caller_ids.push(edge.from);
        }
        let callers_full = self.materialize(caller_ids);

        // Affected tests (full backward walk, test filter).
        let (set, walk_flags) = self.reachable_set(symbol, Direction::Backward);
        flags.merge(&walk_flags);
        let test_ids: Vec<SymbolId> = set
            .into_iter()
            .filter(|id| {
                self.by_id
                    .get(id)
                    .map(|s| s.characteristics.test)
                    .unwrap_or(false)
            })
            .collect();
        let tests_full = self.materialize(test_ids);

        let total = callers_full.len() + tests_full.len();
        let (direct_callers, callers_trunc) = truncate(callers_full, limit);
        let (affected_tests, tests_trunc) = truncate(tests_full, limit);

        let env = Envelope::exact(Impact {
            direct_callers,
            affected_tests,
        })
        .with_total(total)
        .with_truncated(callers_trunc || tests_trunc);
        flags.apply_to(env)
    }
}

/// Truncate a vector to `limit`, returning it and whether anything was dropped.
fn truncate<T>(mut items: Vec<T>, limit: usize) -> (Vec<T>, bool) {
    if items.len() > limit {
        items.truncate(limit);
        (items, true)
    } else {
        (items, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Characteristics, Edge, EdgeKind, Index, Span, Symbol, SymbolId};

    // ── builders ────────────────────────────────────────────────────────────

    fn chars() -> Characteristics {
        Characteristics {
            test: false,
            public: true,
            is_async: false,
            generic: false,
            foreign: false,
            uses_unsafe: false,
        }
    }

    fn sym(fq: &str) -> Symbol {
        Symbol {
            id: SymbolId::from_fq_path(fq),
            fq_path: fq.to_string(),
            crate_name: fq.split("::").next().unwrap_or("").to_string(),
            span: Span {
                file: "src/lib.rs".to_string(),
                line_start: 1,
                line_end: 2,
            },
            characteristics: chars(),
        }
    }

    fn sym_with(fq: &str, mutate: impl FnOnce(&mut Characteristics)) -> Symbol {
        let mut s = sym(fq);
        mutate(&mut s.characteristics);
        s
    }

    fn id(fq: &str) -> SymbolId {
        SymbolId::from_fq_path(fq)
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            from: id(from),
            to: id(to),
            kind,
        }
    }

    fn stat(from: &str, to: &str) -> Edge {
        edge(from, to, EdgeKind::Static)
    }

    fn index(symbols: Vec<Symbol>, edges: Vec<Edge>) -> Index {
        Index {
            schema_version: crate::schema::SCHEMA_VERSION,
            symbols,
            edges,
        }
    }

    fn fq_paths(syms: &[Symbol]) -> Vec<String> {
        syms.iter().map(|s| s.fq_path.clone()).collect()
    }

    // ── C1 resolve ───────────────────────────────────────────────────────────

    #[test]
    fn resolve_returns_all_candidates_never_picks_one() {
        let idx = index(
            vec![
                sym("parser::normalize_token"),
                sym("parser::normalize_line"),
                sym("parser::run_generic"),
            ],
            vec![],
        );
        let g = Graph::new(&idx);
        let env = g.resolve("normalize", 100);
        assert_eq!(
            fq_paths(&env.data),
            vec!["parser::normalize_line", "parser::normalize_token"],
            "both normalize_* candidates returned, sorted by fq_path",
        );
        assert_eq!(env.total, 2);
        assert!(!env.truncated);
    }

    #[test]
    fn resolve_is_case_insensitive_and_empty_query_matches_nothing() {
        let idx = index(vec![sym("parser::NormalizeToken")], vec![]);
        let g = Graph::new(&idx);
        assert_eq!(g.resolve("normalizetoken", 10).data.len(), 1);
        assert_eq!(g.resolve("", 10).data.len(), 0);
        assert_eq!(g.resolve("", 10).total, 0);
    }

    #[test]
    fn resolve_reports_total_when_truncated() {
        let idx = index(
            vec![
                sym("a::foo1"),
                sym("a::foo2"),
                sym("a::foo3"),
                sym("a::foo4"),
            ],
            vec![],
        );
        let g = Graph::new(&idx);
        let env = g.resolve("foo", 2);
        assert_eq!(env.data.len(), 2, "output bounded to limit");
        assert_eq!(env.total, 4, "total reports the full match count");
        assert!(env.truncated);
    }

    // ── C2 direct calls ───────────────────────────────────────────────────────

    #[test]
    fn direct_calls_finds_callers_and_callees() {
        // a -> b -> c ;  d -> b
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![stat("k::a", "k::b"), stat("k::b", "k::c"), stat("k::d", "k::b")],
        );
        let g = Graph::new(&idx);
        let env = g.direct_calls(id("k::b"), 100);
        assert_eq!(fq_paths(&env.data.callers), vec!["k::a", "k::d"]);
        assert_eq!(fq_paths(&env.data.callees), vec!["k::c"]);
        assert_eq!(env.total, 3);
        assert!(!env.truncated);
        assert!(env.over_approximated.is_none());
        assert!(!env.boundary_applies);
    }

    // ── C3 reachability ───────────────────────────────────────────────────────

    #[test]
    fn reachability_forward_is_transitive_and_excludes_start() {
        // a -> b -> c -> d
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![stat("k::a", "k::b"), stat("k::b", "k::c"), stat("k::c", "k::d")],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 100);
        assert_eq!(fq_paths(&env.data), vec!["k::b", "k::c", "k::d"]);
        assert_eq!(env.total, 3);
    }

    #[test]
    fn reachability_backward_is_transitive() {
        // a -> b -> c -> d ; who reaches d? a, b, c
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![stat("k::a", "k::b"), stat("k::b", "k::c"), stat("k::c", "k::d")],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::d"), Direction::Backward, 100);
        assert_eq!(fq_paths(&env.data), vec!["k::a", "k::b", "k::c"]);
        assert_eq!(env.total, 3);
    }

    #[test]
    fn reachability_reports_total_when_truncated() {
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![stat("k::a", "k::b"), stat("k::a", "k::c"), stat("k::a", "k::d")],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 2);
        assert_eq!(env.data.len(), 2, "output bounded to limit");
        assert_eq!(env.total, 3, "total is the full reachable count");
        assert!(env.truncated);
    }

    // ── over-approximation (Q2) ────────────────────────────────────────────────

    #[test]
    fn virtual_edge_flips_over_approximation() {
        // a --virtual--> b : reaching b crosses dyn dispatch
        let idx = index(
            vec![sym("k::a"), sym("k::b")],
            vec![edge("k::a", "k::b", EdgeKind::Virtual)],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 100);
        match env.over_approximated {
            Some(Reason::DynDispatch {
                ref trait_path,
                implementor_count,
            }) => {
                assert_eq!(trait_path, DYN_TRAIT_MARKER);
                assert_eq!(implementor_count, 1, "one distinct virtual callee");
            }
            other => panic!("expected DynDispatch over-approximation, got {other:?}"),
        }
    }

    #[test]
    fn static_only_walk_is_not_over_approximated() {
        let idx = index(
            vec![sym("k::a"), sym("k::b")],
            vec![stat("k::a", "k::b")],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 100);
        assert!(env.over_approximated.is_none());
    }

    // ── boundary ────────────────────────────────────────────────────────────

    #[test]
    fn edge_to_foreign_symbol_flips_boundary() {
        let idx = index(
            vec![
                sym("k::a"),
                sym_with("libc::write", |c| c.foreign = true),
            ],
            vec![stat("k::a", "libc::write")],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 100);
        assert!(env.boundary_applies, "crossing into a foreign symbol is a boundary");
    }

    #[test]
    fn edge_to_absent_symbol_flips_boundary_and_is_a_leaf() {
        // Edge targets an id with no Symbol in the table (third-party crate the
        // index filtered out). It is a boundary and cannot be walked into.
        let idx = index(vec![sym("k::a")], vec![stat("k::a", "external::thing")]);
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 100);
        assert!(env.boundary_applies);
        assert!(env.data.is_empty(), "absent target yields no walkable symbol");
        assert_eq!(env.total, 0);
    }

    // ── C4 call paths ─────────────────────────────────────────────────────────

    #[test]
    fn call_paths_enumerates_both_routes() {
        // a -> b -> d ; a -> c -> d : two distinct paths a..d
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![
                stat("k::a", "k::b"),
                stat("k::b", "k::d"),
                stat("k::a", "k::c"),
                stat("k::c", "k::d"),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::d"), 10, 100);
        assert_eq!(env.total, 2, "two paths a->d");
        let routes: Vec<Vec<String>> = env
            .data
            .iter()
            .map(|p| p.nodes.iter().map(|s| s.fq_path.clone()).collect())
            .collect();
        assert_eq!(
            routes,
            vec![
                vec!["k::a", "k::b", "k::d"],
                vec!["k::a", "k::c", "k::d"],
            ],
        );
        assert!(!env.truncated);
    }

    #[test]
    fn call_paths_caps_count_and_reports_truncation() {
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![
                stat("k::a", "k::b"),
                stat("k::b", "k::d"),
                stat("k::a", "k::c"),
                stat("k::c", "k::d"),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::d"), 10, 1);
        assert_eq!(env.data.len(), 1);
        assert!(env.truncated, "path count cap hit");
    }

    #[test]
    fn call_paths_depth_bound_reports_truncation() {
        // a -> b -> c -> d ; depth 1 cannot reach d, and a branch is cut
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![stat("k::a", "k::b"), stat("k::b", "k::c"), stat("k::c", "k::d")],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::d"), 1, 100);
        assert!(env.data.is_empty());
        assert!(env.truncated, "depth bound cut a branch before reaching target");
    }

    #[test]
    fn call_paths_carries_virtual_flag_from_used_edge() {
        let idx = index(
            vec![sym("k::a"), sym("k::b")],
            vec![edge("k::a", "k::b", EdgeKind::Virtual)],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::b"), 10, 100);
        assert_eq!(env.total, 1);
        assert!(env.over_approximated.is_some());
    }

    #[test]
    fn call_paths_exactly_max_paths_is_not_truncated() {
        // Exactly two paths a..d; asking for max_paths=2 must return both and
        // report truncated=false — the count equals the cap, nothing was dropped.
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![
                stat("k::a", "k::b"),
                stat("k::b", "k::d"),
                stat("k::a", "k::c"),
                stat("k::c", "k::d"),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::d"), 10, 2);
        assert_eq!(env.data.len(), 2, "both paths returned");
        assert_eq!(env.total, 2);
        assert!(
            !env.truncated,
            "count equals the cap but nothing was dropped, so not truncated",
        );
    }

    #[test]
    fn call_paths_over_max_paths_truncates_and_returns_exactly_cap() {
        // Three paths a..d (via b, c, e); max_paths=2 must return exactly 2 and
        // report truncated=true.
        let idx = index(
            vec![
                sym("k::a"),
                sym("k::b"),
                sym("k::c"),
                sym("k::d"),
                sym("k::e"),
            ],
            vec![
                stat("k::a", "k::b"),
                stat("k::b", "k::d"),
                stat("k::a", "k::c"),
                stat("k::c", "k::d"),
                stat("k::a", "k::e"),
                stat("k::e", "k::d"),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::d"), 10, 2);
        assert_eq!(env.data.len(), 2, "returns exactly max_paths");
        assert_eq!(env.total, 2, "total is the returned count");
        assert!(env.truncated, "a third path exists beyond the cap");
    }

    #[test]
    fn call_paths_truncated_still_flags_virtual_edge_on_dropped_path() {
        // Two paths a..d. The kept path (a->b->d, sorts first) is all-static; the
        // dropped path (a->c->d) crosses a Virtual edge c->d. With max_paths=1 the
        // virtual-carrying path is dropped, yet over-approximation must still be
        // reported — flags are accumulated during the walk, not off survivors.
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![
                stat("k::a", "k::b"),
                stat("k::b", "k::d"),
                stat("k::a", "k::c"),
                edge("k::c", "k::d", EdgeKind::Virtual),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::d"), 10, 1);
        assert_eq!(env.data.len(), 1, "only max_paths returned");
        assert_eq!(
            fq_paths(&env.data[0].nodes),
            vec!["k::a", "k::b", "k::d"],
            "the kept path is the all-static one",
        );
        assert!(env.truncated, "a second path was dropped");
        assert!(
            env.over_approximated.is_some(),
            "the dropped path's virtual edge must still surface as over-approximation",
        );
    }

    // ── cycles ────────────────────────────────────────────────────────────────

    #[test]
    fn cycle_does_not_infinite_loop_reachability() {
        // a -> b -> c -> a  (cycle)
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c")],
            vec![stat("k::a", "k::b"), stat("k::b", "k::c"), stat("k::c", "k::a")],
        );
        let g = Graph::new(&idx);
        let env = g.reachability(id("k::a"), Direction::Forward, 100);
        // From a, the whole cycle is reachable, including a itself via the loop.
        assert_eq!(fq_paths(&env.data), vec!["k::a", "k::b", "k::c"]);
    }

    #[test]
    fn cycle_does_not_infinite_loop_paths() {
        // a -> b -> a (cycle) and a -> b is the only route to b.
        let idx = index(
            vec![sym("k::a"), sym("k::b")],
            vec![stat("k::a", "k::b"), stat("k::b", "k::a")],
        );
        let g = Graph::new(&idx);
        let env = g.call_paths(id("k::a"), id("k::b"), 10, 100);
        // Exactly one simple path a->b; the back-edge cannot extend it.
        assert_eq!(env.total, 1);
        assert_eq!(
            env.data[0].nodes.iter().map(|s| s.fq_path.clone()).collect::<Vec<_>>(),
            vec!["k::a", "k::b"],
        );
    }

    // ── C5 affected tests ───────────────────────────────────────────────────────

    #[test]
    fn affected_tests_picks_only_test_symbols() {
        // test_a -> mid -> target ; plain_b -> target ; other_test -> elsewhere
        let idx = index(
            vec![
                sym_with("k::test_a", |c| c.test = true),
                sym("k::mid"),
                sym("k::target"),
                sym("k::plain_b"),
                sym_with("k::other_test", |c| c.test = true),
                sym("k::elsewhere"),
            ],
            vec![
                stat("k::test_a", "k::mid"),
                stat("k::mid", "k::target"),
                stat("k::plain_b", "k::target"),
                stat("k::other_test", "k::elsewhere"),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.affected_tests(id("k::target"), 100);
        assert_eq!(
            fq_paths(&env.data),
            vec!["k::test_a"],
            "only the test that transitively reaches target, not plain_b nor other_test",
        );
        assert_eq!(env.total, 1);
    }

    #[test]
    fn affected_tests_through_virtual_edge_is_over_approximated() {
        // The guiding-example shape: a test reaches the target only through a
        // dyn-dispatch (virtual) edge. It must appear AND be flagged Q2.
        let idx = index(
            vec![
                sym_with("k::dyn_test", |c| c.test = true),
                sym("k::run_generic"),
                sym("k::normalize_token"),
            ],
            vec![
                stat("k::dyn_test", "k::run_generic"),
                edge("k::run_generic", "k::normalize_token", EdgeKind::Virtual),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.affected_tests(id("k::normalize_token"), 100);
        assert_eq!(fq_paths(&env.data), vec!["k::dyn_test"]);
        assert!(
            env.over_approximated.is_some(),
            "reaching through dyn dispatch over-approximates the answer",
        );
    }

    // ── C6 reachable unsafe ─────────────────────────────────────────────────────

    #[test]
    fn reachable_unsafe_picks_only_unsafe_symbols() {
        // a -> b(unsafe) -> c ; a -> d
        let idx = index(
            vec![
                sym("k::a"),
                sym_with("k::b", |c| c.uses_unsafe = true),
                sym("k::c"),
                sym("k::d"),
            ],
            vec![stat("k::a", "k::b"), stat("k::b", "k::c"), stat("k::a", "k::d")],
        );
        let g = Graph::new(&idx);
        let env = g.reachable_unsafe(id("k::a"), 100);
        assert_eq!(fq_paths(&env.data), vec!["k::b"]);
        assert_eq!(env.total, 1);
    }

    // ── C7 impact ─────────────────────────────────────────────────────────────

    #[test]
    fn impact_combines_direct_callers_and_affected_tests() {
        // caller -> target ; test_x -> caller -> target
        let idx = index(
            vec![
                sym("k::caller"),
                sym("k::target"),
                sym_with("k::test_x", |c| c.test = true),
            ],
            vec![stat("k::caller", "k::target"), stat("k::test_x", "k::caller")],
        );
        let g = Graph::new(&idx);
        let env = g.impact(id("k::target"), 100);
        assert_eq!(fq_paths(&env.data.direct_callers), vec!["k::caller"]);
        assert_eq!(fq_paths(&env.data.affected_tests), vec!["k::test_x"]);
        assert_eq!(env.total, 2, "one caller + one test");
    }

    #[test]
    fn impact_merges_flags_from_both_halves() {
        // A virtual edge on the backward test walk must surface on the impact
        // envelope even though the direct-caller scan is all static.
        let idx = index(
            vec![
                sym("k::caller"),
                sym("k::target"),
                sym_with("k::dyn_test", |c| c.test = true),
            ],
            vec![
                stat("k::caller", "k::target"),
                edge("k::dyn_test", "k::caller", EdgeKind::Virtual),
            ],
        );
        let g = Graph::new(&idx);
        let env = g.impact(id("k::target"), 100);
        assert!(env.over_approximated.is_some());
    }
}
