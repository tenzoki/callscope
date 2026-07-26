//! The loaded index and the pure query handlers behind the eight MCP tools.
//!
//! This module holds everything that is *not* the MCP wire protocol: loading
//! `index.bin` + `manifest.json` from disk, running the staleness fingerprint,
//! resolving a symbol name to a [`SymbolId`], and calling the `callscope-core`
//! query functions. Keeping it separate from `tools.rs` means the whole answer
//! path is testable without standing up an MCP client — the integration test
//! builds an [`IndexState`] and calls these methods directly.
//!
//! # Why the payloads are re-wrapped
//!
//! `callscope-core`'s query payloads ([`DirectCalls`], [`CallPath`], [`Impact`])
//! derive no `Serialize` — they are internal query types. This crate cannot edit
//! core, so it defines serializable mirrors ([`DirectCallsOut`], [`CallPathOut`],
//! [`ImpactOut`]) and remaps the envelope onto them, preserving every uncertainty
//! flag ([`remap_envelope`]). `Symbol` itself *is* serializable, so the mirrors
//! only re-shape the container, never the symbols inside.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use callscope_core::fingerprint::diverged_files;
use callscope_core::mermaid::render_neighborhood;
use callscope_core::query::{CallPath, DirectCalls, Direction, Graph, Impact};
use callscope_core::{Envelope, Index, Manifest, StaleInfo, Symbol, SymbolId, SCHEMA_VERSION};

/// Default cap on candidate / result-set size when the caller passes no `limit`.
pub const DEFAULT_LIMIT: usize = 200;
/// Default maximum edges per enumerated call path (C4).
pub const DEFAULT_MAX_DEPTH: usize = 8;
/// Default cap on the number of enumerated call paths (C4).
pub const DEFAULT_MAX_PATHS: usize = 64;
/// Default undirected radius for the neighborhood graph (C8).
pub const DEFAULT_GRAPH_DEPTH: usize = 2;
/// Default cap on drawn nodes in the neighborhood graph (C8).
pub const DEFAULT_NODE_LIMIT: usize = 60;

/// A boxed error that is `Send + Sync`, so it crosses the async serve boundary.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

// ── serializable mirrors of the core query payloads ─────────────────────────

/// Serializable mirror of [`callscope_core::query::DirectCalls`] (C2).
#[derive(Serialize, Debug, Clone)]
pub struct DirectCallsOut {
    pub callers: Vec<Symbol>,
    pub callees: Vec<Symbol>,
}

/// Serializable mirror of [`callscope_core::query::CallPath`] (C4).
#[derive(Serialize, Debug, Clone)]
pub struct CallPathOut {
    pub nodes: Vec<Symbol>,
}

/// Serializable mirror of [`callscope_core::query::Impact`] (C7).
#[derive(Serialize, Debug, Clone)]
pub struct ImpactOut {
    pub direct_callers: Vec<Symbol>,
    pub affected_tests: Vec<Symbol>,
}

/// Rebuild an [`Envelope`] over a new payload, carrying every uncertainty field
/// across unchanged. Used to move a core query payload into its serializable
/// mirror without losing `stale`, `over_approximated`, `truncated`, `total`, or
/// `boundary_applies`.
fn remap_envelope<A, B>(env: Envelope<A>, data: B) -> Envelope<B> {
    Envelope {
        data,
        stale: env.stale,
        over_approximated: env.over_approximated,
        truncated: env.truncated,
        total: env.total,
        boundary_applies: env.boundary_applies,
    }
}

// ── symbol resolution outcome (Q3) ───────────────────────────────────────────

/// The result of resolving a symbol name or fragment (C1) to a single target.
///
/// A tool never guesses when a name is ambiguous (Q3): it hands back the
/// candidate set instead of an answer, and the caller disambiguates by passing a
/// more specific fragment, the exact fully-qualified path, or the numeric
/// [`SymbolId`].
pub enum Resolved {
    /// Exactly one symbol matched — proceed with the query.
    One(SymbolId),
    /// More than one symbol matched. The envelope carries the candidate set so
    /// the caller can pick one; `total` reports how many matched.
    Ambiguous(Envelope<Vec<Symbol>>),
    /// No symbol matched. The envelope's `data` is the empty candidate list.
    NotFound(Envelope<Vec<Symbol>>),
}

/// The loaded index, its staleness manifest, and the workspace it describes.
///
/// Constructed once by [`IndexState::load`] at server start-up. The query
/// methods below build a fresh [`Graph`] view per call: a `Graph` borrows the
/// index, so storing it beside the owned index would be self-referential, and
/// re-deriving the adjacency lists per request is negligible at workspace scale.
pub struct IndexState {
    index: Index,
    manifest: Manifest,
    workspace_root: PathBuf,
}

impl IndexState {
    /// Load `index.bin` and `manifest.json` from `<workspace_root>/.callscope/`.
    ///
    /// Both artifacts are read as JSON (see the on-disk-format decision record).
    /// The index's `schema_version` is checked against the compiled
    /// [`SCHEMA_VERSION`] and a mismatch is rejected rather than misparsed.
    pub fn load(workspace_root: impl Into<PathBuf>) -> Result<Self, BoxError> {
        let workspace_root = workspace_root.into();
        let dir = workspace_root.join(".callscope");

        let index_path = dir.join("index.bin");
        let index_bytes = fs::read(&index_path).map_err(|e| {
            format!("cannot read index at {}: {e}", index_path.display())
        })?;
        let index: Index = serde_json::from_slice(&index_bytes).map_err(|e| {
            format!("cannot parse index at {}: {e}", index_path.display())
        })?;
        if index.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "index schema version {} does not match this build's {} — re-index with a matching callscope-index",
                index.schema_version, SCHEMA_VERSION
            )
            .into());
        }

        let manifest_path = dir.join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path).map_err(|e| {
            format!("cannot read manifest at {}: {e}", manifest_path.display())
        })?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
            format!("cannot parse manifest at {}: {e}", manifest_path.display())
        })?;

        Ok(IndexState {
            index,
            manifest,
            workspace_root,
        })
    }

    /// Run the staleness fingerprint (Q6): which indexed sources diverged from
    /// the manifest since the index was built.
    ///
    /// Returns `Ok(None)` when the index is current and `Ok(Some(..))` with the
    /// diverged files otherwise. Called on every tool request by the tool layer,
    /// which attaches the result to the outgoing envelope.
    pub fn compute_stale(&self) -> std::io::Result<Option<StaleInfo>> {
        let diverged = diverged_files(&self.manifest, &self.workspace_root)?;
        Ok(if diverged.is_empty() {
            None
        } else {
            Some(StaleInfo::new(diverged))
        })
    }

    // ── C1: resolution ──────────────────────────────────────────────────────

    /// Resolve a name or fragment to the candidate set (C1). Returns the
    /// envelope straight from the query layer; the tool wrapper attaches
    /// staleness.
    pub fn resolve_symbol(&self, query: &str, limit: usize) -> Envelope<Vec<Symbol>> {
        Graph::new(&self.index).resolve(query, limit)
    }

    /// Resolve a name to a single [`SymbolId`], or hand back candidates (Q3).
    ///
    /// Disambiguation precedence, most specific first:
    /// 1. A numeric [`SymbolId`] (the raw `u64`) that names a symbol in the index.
    /// 2. An exact fully-qualified-path match (unambiguous even if it is also a
    ///    substring of other paths).
    /// 3. Otherwise the C1 fragment search: one match resolves, zero is
    ///    `NotFound`, many is `Ambiguous`.
    fn resolve_one(&self, name: &str, limit: usize) -> Resolved {
        // 1. Exact numeric SymbolId.
        if let Ok(raw) = name.parse::<u64>() {
            let sid = SymbolId(raw);
            if self.index.symbols.iter().any(|s| s.id == sid) {
                return Resolved::One(sid);
            }
        }
        // 2. Exact fully-qualified path.
        if let Some(sym) = self.index.symbols.iter().find(|s| s.fq_path == name) {
            return Resolved::One(sym.id);
        }
        // 3. Fragment search.
        let env = Graph::new(&self.index).resolve(name, limit);
        match env.data.len() {
            1 => Resolved::One(env.data[0].id),
            0 => Resolved::NotFound(env),
            _ => Resolved::Ambiguous(env),
        }
    }

    // ── C2–C8: per-capability handlers keyed by an already-resolved id ────────

    /// C2 — direct callers and callees.
    pub fn direct_calls(&self, symbol: SymbolId, limit: usize) -> Envelope<DirectCallsOut> {
        let env = Graph::new(&self.index).direct_calls(symbol, limit);
        let DirectCalls { callers, callees } = env.data.clone();
        remap_envelope(env, DirectCallsOut { callers, callees })
    }

    /// C3 — transitive reachability in `dir`.
    pub fn reachability(
        &self,
        symbol: SymbolId,
        dir: Direction,
        limit: usize,
    ) -> Envelope<Vec<Symbol>> {
        Graph::new(&self.index).reachability(symbol, dir, limit)
    }

    /// C4 — enumerated simple call paths from `from` to `to`.
    pub fn call_paths(
        &self,
        from: SymbolId,
        to: SymbolId,
        max_depth: usize,
        max_paths: usize,
    ) -> Envelope<Vec<CallPathOut>> {
        let env = Graph::new(&self.index).call_paths(from, to, max_depth, max_paths);
        let paths: Vec<CallPathOut> = env
            .data
            .iter()
            .map(|p: &CallPath| CallPathOut {
                nodes: p.nodes.clone(),
            })
            .collect();
        remap_envelope(env, paths)
    }

    /// C5 — tests that transitively reach `symbol`.
    pub fn affected_tests(&self, symbol: SymbolId, limit: usize) -> Envelope<Vec<Symbol>> {
        Graph::new(&self.index).affected_tests(symbol, limit)
    }

    /// C6 — unsafe-using symbols reachable forward from `symbol`.
    pub fn reachable_unsafe(&self, symbol: SymbolId, limit: usize) -> Envelope<Vec<Symbol>> {
        Graph::new(&self.index).reachable_unsafe(symbol, limit)
    }

    /// C7 — combined change impact: direct callers plus affected tests.
    pub fn impact(&self, symbol: SymbolId, limit: usize) -> Envelope<ImpactOut> {
        let env = Graph::new(&self.index).impact(symbol, limit);
        let Impact {
            direct_callers,
            affected_tests,
        } = env.data.clone();
        remap_envelope(
            env,
            ImpactOut {
                direct_callers,
                affected_tests,
            },
        )
    }

    /// C8 — Mermaid neighborhood graph around `symbol`.
    pub fn neighborhood_graph(
        &self,
        symbol: SymbolId,
        depth: usize,
        node_limit: usize,
    ) -> Envelope<String> {
        render_neighborhood(&self.index, symbol, depth, node_limit)
    }

    /// C4 by name — resolve both endpoints, then enumerate paths. Either end
    /// being ambiguous or unknown short-circuits to that end's candidate set
    /// (Q3), so the caller learns which name it must make specific.
    pub fn call_paths_by_name(
        &self,
        from: &str,
        to: &str,
        max_depth: usize,
        max_paths: usize,
        limit: usize,
    ) -> Result<Envelope<Vec<CallPathOut>>, Envelope<Vec<Symbol>>> {
        let from_id = match self.resolve_one(from, limit) {
            Resolved::One(id) => id,
            Resolved::Ambiguous(env) | Resolved::NotFound(env) => return Err(env),
        };
        let to_id = match self.resolve_one(to, limit) {
            Resolved::One(id) => id,
            Resolved::Ambiguous(env) | Resolved::NotFound(env) => return Err(env),
        };
        Ok(self.call_paths(from_id, to_id, max_depth, max_paths))
    }

    /// Resolve a name and run `f` on the single matching id, or return the
    /// candidate/not-found envelope serialized as `Vec<Symbol>`.
    ///
    /// This is the shared disambiguation path (Q3) for every symbol-taking tool.
    /// The `Ok` branch is the resolved answer; the `Err` branch is the candidate
    /// set the caller must choose from — both are envelopes, both are serialized
    /// identically by the tool layer.
    pub fn resolve_then<T>(
        &self,
        name: &str,
        limit: usize,
        f: impl FnOnce(SymbolId) -> Envelope<T>,
    ) -> Result<Envelope<T>, Envelope<Vec<Symbol>>> {
        match self.resolve_one(name, limit) {
            Resolved::One(id) => Ok(f(id)),
            Resolved::Ambiguous(env) | Resolved::NotFound(env) => Err(env),
        }
    }
}

/// Attach staleness to any envelope, then serialize it to a compact JSON string.
///
/// This is the single serialization point for every tool answer: the query layer
/// never sets `stale` (it cannot see the disk), so the tool layer stamps it here
/// (Q6) just before the envelope goes on the wire.
pub fn envelope_to_json<T: Serialize>(
    env: Envelope<T>,
    stale: Option<StaleInfo>,
) -> Result<String, BoxError> {
    let env = match stale {
        Some(info) => env.with_stale(info),
        None => env,
    };
    Ok(serde_json::to_string(&env)?)
}
