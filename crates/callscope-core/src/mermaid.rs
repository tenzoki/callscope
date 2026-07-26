//! C8 — render the bounded call-graph neighborhood around a symbol as Mermaid
//! `flowchart` text the agent reads inline (settled decision 3: Mermaid, no
//! external renderer).
//!
//! # What "neighborhood" means here
//!
//! The neighborhood is every workspace symbol within `depth` *undirected* hops
//! of the focus — its callers and its callees, out to the requested radius.
//! Traversal is not reimplemented: each ring is expanded with
//! [`Graph::direct_calls`], so this module leans on the same adjacency and
//! edge-flag logic the rest of the query engine uses. This module only *lays
//! out* the collected subgraph as text.
//!
//! # Envelope semantics (Q4 / Q2 / boundary)
//!
//! - **Bounded output (Q4).** `node_limit` caps how many workspace symbols are
//!   drawn. The full neighborhood is always collected first, so `total` is the
//!   true symbol count; when the cap cuts it short, `truncated` is set and a
//!   distinct truncation note is drawn into the graph.
//! - **Over-approximation (Q2).** If any drawn edge is [`EdgeKind::Virtual`]
//!   (dyn dispatch), the envelope's `over_approximated` is set — the same signal
//!   the C1–C7 queries carry — and the edge is drawn dashed and labelled `dyn`.
//! - **Boundary.** An edge leaving the workspace (into a filtered third-party
//!   crate — an absent target — or a `foreign`/FFI symbol) is drawn thick and
//!   labelled `boundary`, and `boundary_applies` is set. Absent targets are
//!   drawn as synthetic boundary nodes so the edge has somewhere to land.
//!
//! # Mermaid v11.13.0 compatibility
//!
//! The v11 parser is strict, so safe output is produced *by construction*:
//! - Node ids are always the generated forms `n<i>` (workspace symbols),
//!   `b<i>` (synthetic boundary targets), or `trunc_note` — never a raw
//!   `fq_path`. That keeps reserved words (`graph`, `end`, `class`, …) and the
//!   special characters in Rust paths (`::`, `<`, `>`, `(`, spaces) out of the
//!   id position entirely; they live only inside the quoted label.
//! - Labels never contain a literal `\n` (a v11 syntax error); this renderer
//!   inserts no line breaks into labels at all, and HTML-escapes `<`, `>`, `&`
//!   and `"` so a generic path like `Vec<String>` cannot break the quoting.

use std::collections::{BTreeSet, HashMap};

use crate::envelope::{Envelope, Reason};
use crate::query::Graph;
use crate::schema::{Edge, EdgeKind, Index, Symbol, SymbolId};

/// The `trait_path` reported when a rendered neighborhood crosses a virtual
/// edge. v1's [`Edge`] does not carry the dispatched trait's identity, so the
/// label is generic — mirroring [`crate::query::DYN_TRAIT_MARKER`].
const DYN_TRAIT_MARKER: &str = "<dyn dispatch>";

/// Render the bounded neighborhood around `focus` out to `depth` hops (callers
/// and callees), capping the drawn workspace symbols at `node_limit`.
///
/// Returns the Mermaid `flowchart` text wrapped in the standard [`Envelope`]:
/// `total` is the full neighborhood symbol count, `truncated` is set when the
/// cap cut it, `over_approximated` is set when a drawn edge is virtual, and
/// `boundary_applies` is set when a drawn edge leaves the workspace.
pub fn render_neighborhood(
    index: &Index,
    focus: SymbolId,
    depth: usize,
    node_limit: usize,
) -> Envelope<String> {
    let by_id: HashMap<SymbolId, &Symbol> =
        index.symbols.iter().map(|s| (s.id, s)).collect();

    // A focus that names no symbol yields an (otherwise valid) empty flowchart
    // with a comment, rather than a panic or a misleading graph.
    if !by_id.contains_key(&focus) {
        let text = format!(
            "flowchart TD\n    %% focus symbol id {} not found in index\n",
            focus.0
        );
        return Envelope::exact(text).with_total(0);
    }

    let graph = Graph::new(index);

    // ── collect the neighborhood (reusing Graph::direct_calls per ring) ──────
    let order = collect_neighborhood(&graph, &by_id, focus, depth);
    let total = order.len();
    let truncated = total > node_limit;

    // Keep the first `node_limit` in BFS order; focus is order[0], so it always
    // survives a non-zero cap.
    let kept: Vec<SymbolId> = order.into_iter().take(node_limit).collect();
    let kept_set: BTreeSet<SymbolId> = kept.iter().copied().collect();

    // Stable `n<i>` id per kept symbol, assigned in BFS order.
    let node_id: HashMap<SymbolId, String> = kept
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, format!("n{i}")))
        .collect();

    // ── render edges (deterministic order over a sorted edge view) ───────────
    let mut edges_sorted: Vec<&Edge> = index.edges.iter().collect();
    edges_sorted.sort_by(|a, b| edge_key(a, &by_id).cmp(&edge_key(b, &by_id)));

    let mut edge_lines: Vec<String> = Vec::new();
    // Synthetic boundary nodes for absent (filtered-out) targets, in first-seen
    // order so the `b<i>` numbering is deterministic.
    let mut boundary_nodes: Vec<(String, String)> = Vec::new();
    let mut boundary_of: HashMap<SymbolId, String> = HashMap::new();
    let mut over_approximated = false;
    let mut boundary_applies = false;

    for edge in edges_sorted {
        let Some(from_node) = node_id.get(&edge.from) else {
            continue; // source not drawn — edge is outside the window
        };
        let to_boundary = is_boundary_target(edge.to, &by_id);

        if let Some(to_node) = node_id.get(&edge.to) {
            // Both endpoints are drawn workspace symbols.
            edge_lines.push(edge_line(from_node, to_node, edge.kind, to_boundary));
        } else if to_boundary {
            // Target left the workspace and is not itself drawn: give it a
            // synthetic boundary node to land on.
            let to_node = boundary_of.entry(edge.to).or_insert_with(|| {
                let name = format!("b{}", boundary_nodes.len());
                let label = by_id
                    .get(&edge.to)
                    .map(|s| s.fq_path.clone())
                    .unwrap_or_else(|| "external".to_string());
                boundary_nodes.push((name.clone(), label));
                name
            });
            edge_lines.push(edge_line(from_node, to_node, edge.kind, true));
        } else {
            continue; // target is a workspace symbol outside the window/cap
        }

        if edge.kind == EdgeKind::Virtual {
            over_approximated = true;
        }
        if to_boundary {
            boundary_applies = true;
        }
    }

    // ── assemble the flowchart text ──────────────────────────────────────────
    let mut out = String::from("flowchart TD\n");

    for id in &kept {
        let node = &node_id[id];
        let label = escape_label(&by_id[id].fq_path);
        if *id == focus {
            out.push_str(&format!("    {node}[\"{label}\"]:::focus\n"));
        } else if is_boundary_target(*id, &by_id) {
            out.push_str(&format!("    {node}[\"{label}\"]:::boundary\n"));
        } else {
            out.push_str(&format!("    {node}[\"{label}\"]\n"));
        }
    }

    for (node, label) in &boundary_nodes {
        let label = escape_label(label);
        out.push_str(&format!("    {node}[\"{label}\"]:::boundary\n"));
    }

    if truncated {
        let hidden = total - kept.len();
        out.push_str(&format!(
            "    trunc_note[\"+{hidden} more not shown (total {total})\"]:::trunc\n"
        ));
    }

    for line in &edge_lines {
        out.push_str(line);
    }

    // classDefs last: they style the marker classes used above. `focus` is the
    // queried symbol, `boundary` a workspace-edge crossing, `trunc` the note.
    out.push_str("    classDef focus fill:#ffd54f,stroke:#333,stroke-width:2px;\n");
    out.push_str("    classDef boundary fill:#eeeeee,stroke:#999,stroke-dasharray:4 3;\n");
    out.push_str("    classDef trunc fill:#ffffff,stroke:#c62828,stroke-dasharray:2 2;\n");

    let mut env = Envelope::exact(out)
        .with_total(total)
        .with_truncated(truncated)
        .with_boundary(boundary_applies);
    if over_approximated {
        // implementor_count mirrors the query engine: distinct virtual targets
        // drawn. Counting them here would require re-walking; the boolean signal
        // and marker are the contract, so report the drawn virtual-edge targets.
        let virtual_targets = count_virtual_targets(index, &kept_set);
        env = env.with_over_approximation(Reason::dyn_dispatch(DYN_TRAIT_MARKER, virtual_targets));
    }
    env
}

/// BFS the neighborhood out to `depth` undirected hops, returning symbol ids in
/// deterministic BFS order (focus first, each ring sorted by `fq_path`). Boundary
/// targets are not expanded — they are workspace-edge leaves, matching the query
/// engine's treatment.
fn collect_neighborhood(
    graph: &Graph<'_>,
    by_id: &HashMap<SymbolId, &Symbol>,
    focus: SymbolId,
    depth: usize,
) -> Vec<SymbolId> {
    let mut order: Vec<SymbolId> = vec![focus];
    let mut visited: BTreeSet<SymbolId> = BTreeSet::new();
    visited.insert(focus);
    let mut frontier: Vec<SymbolId> = vec![focus];

    for _ in 0..depth {
        let mut ring: Vec<SymbolId> = Vec::new();
        for &node in &frontier {
            if is_boundary_target(node, by_id) {
                continue; // do not expand a workspace-edge leaf
            }
            let dc = graph.direct_calls(node, usize::MAX);
            for sym in dc.data.callers.iter().chain(dc.data.callees.iter()) {
                if visited.insert(sym.id) {
                    ring.push(sym.id);
                }
            }
        }
        // Sort the ring for a deterministic layout and a deterministic cap.
        ring.sort_by(|a, b| fq_key(*a, by_id).cmp(&fq_key(*b, by_id)));
        order.extend(ring.iter().copied());
        frontier = ring;
        if frontier.is_empty() {
            break;
        }
    }
    order
}

/// Whether crossing to `target` leaves the workspace: mirrors
/// [`crate::query`]'s boundary rule — absent from the symbol table (its crate
/// was filtered out) or present but `foreign` (FFI).
fn is_boundary_target(target: SymbolId, by_id: &HashMap<SymbolId, &Symbol>) -> bool {
    match by_id.get(&target) {
        None => true,
        Some(sym) => sym.characteristics.foreign,
    }
}

/// Count the distinct virtual-edge targets among drawn edges, for the
/// over-approximation `implementor_count`.
fn count_virtual_targets(index: &Index, kept: &BTreeSet<SymbolId>) -> usize {
    let mut targets: BTreeSet<SymbolId> = BTreeSet::new();
    for edge in &index.edges {
        if edge.kind == EdgeKind::Virtual && kept.contains(&edge.from) {
            targets.insert(edge.to);
        }
    }
    targets.len()
}

/// One edge line, styled by kind and boundary status. Boundary styling wins over
/// kind: a boundary crossing is drawn thick and labelled `boundary`; a
/// non-boundary virtual edge is dashed and labelled `dyn`; a plain static edge
/// is a solid arrow.
fn edge_line(from: &str, to: &str, kind: EdgeKind, boundary: bool) -> String {
    if boundary {
        format!("    {from} ==>|boundary| {to}\n")
    } else if kind == EdgeKind::Virtual {
        format!("    {from} -.->|dyn| {to}\n")
    } else {
        format!("    {from} --> {to}\n")
    }
}

/// Sort key for a symbol id: its `fq_path`, or its raw id (as a stable fallback)
/// when the id names no symbol.
fn fq_key(id: SymbolId, by_id: &HashMap<SymbolId, &Symbol>) -> (String, u64) {
    match by_id.get(&id) {
        Some(s) => (s.fq_path.clone(), id.0),
        None => (String::new(), id.0),
    }
}

/// Deterministic sort key for an edge: (from fq_path, to fq_path, kind).
fn edge_key(edge: &Edge, by_id: &HashMap<SymbolId, &Symbol>) -> (String, u64, String, u64, u8) {
    let (ff, fi) = fq_key(edge.from, by_id);
    let (tf, ti) = fq_key(edge.to, by_id);
    let k = match edge.kind {
        EdgeKind::Static => 0,
        EdgeKind::Virtual => 1,
    };
    (ff, fi, tf, ti, k)
}

/// HTML-escape a label so Rust path punctuation (`<`, `>`, `&`) and any stray
/// `"` cannot break the quoted node label under the strict v11 parser. Inserts
/// no line breaks, so no label ever contains a literal `\n`.
fn escape_label(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Characteristics, Edge, EdgeKind, Index, Span, Symbol, SymbolId};

    /// Mermaid v11 reserved words that must never appear as a node id.
    const RESERVED: [&str; 12] = [
        "graph", "end", "subgraph", "class", "classDef", "state", "flowchart", "style", "click",
        "linkStyle", "direction", "default",
    ];

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

    fn index(symbols: Vec<Symbol>, edges: Vec<Edge>) -> Index {
        Index {
            schema_version: crate::schema::SCHEMA_VERSION,
            symbols,
            edges,
        }
    }

    /// Extract the node id from every declaration line (`    <id>["..."]...`).
    fn declared_ids(mermaid: &str) -> Vec<String> {
        mermaid
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let brace = t.find("[\"")?;
                Some(t[..brace].to_string())
            })
            .collect()
    }

    // The guiding-example shape: a test reaches normalize_token only through a
    // dyn-dispatch (virtual) edge, and normalize_token also calls out to a
    // filtered third-party crate (a boundary edge).
    fn guiding_index() -> Index {
        index(
            vec![
                sym_with("parser::dyn_test", |c| c.test = true),
                sym("parser::run_generic"),
                sym("parser::normalize_token"),
            ],
            vec![
                edge("parser::dyn_test", "parser::run_generic", EdgeKind::Static),
                edge("parser::run_generic", "parser::normalize_token", EdgeKind::Virtual),
                // normalize_token -> a symbol absent from the table (third-party
                // crate the index filtered out): a boundary crossing.
                edge("parser::normalize_token", "regex::Regex::new", EdgeKind::Static),
            ],
        )
    }

    #[test]
    fn renders_a_flowchart_with_safe_ids_and_visible_edge_kinds() {
        let idx = guiding_index();
        let env = render_neighborhood(&idx, id("parser::normalize_token"), 2, 50);
        let out = &env.data;

        // It is a flowchart.
        assert!(out.contains("flowchart"), "output must be a mermaid flowchart:\n{out}");

        // Node ids are the safe generated form, and NONE is a reserved word.
        let ids = declared_ids(out);
        assert!(!ids.is_empty(), "expected node declarations:\n{out}");
        for node_id in &ids {
            let safe = (node_id.starts_with('n') || node_id.starts_with('b'))
                && node_id[1..].chars().all(|c| c.is_ascii_digit())
                || node_id == "trunc_note";
            assert!(safe, "node id `{node_id}` is not the safe generated form:\n{out}");
            assert!(
                !RESERVED.contains(&node_id.as_str()),
                "node id `{node_id}` is a mermaid reserved word:\n{out}",
            );
        }

        // The focus symbol is drawn with the focus class, its path only inside
        // the quoted label (never as a bare id).
        assert!(
            out.contains("[\"parser::normalize_token\"]:::focus"),
            "focus node must carry its path in a quoted label and the focus class:\n{out}",
        );

        // A virtual (dyn) edge renders in the dashed, labelled form.
        assert!(
            out.contains("-.->|dyn|"),
            "virtual edge must render dashed and labelled `dyn`:\n{out}",
        );
        // ...and that flips over-approximation on the envelope (Q2).
        assert!(
            env.over_approximated.is_some(),
            "a drawn virtual edge must over-approximate the answer",
        );

        // The boundary crossing renders distinctly and flips the flag.
        assert!(
            out.contains("==>|boundary|"),
            "boundary edge must render thick and labelled `boundary`:\n{out}",
        );
        assert!(env.boundary_applies, "a drawn boundary edge sets boundary_applies");

        // No label contains a literal `\n` (a v11 syntax error): the two-char
        // backslash-n sequence must not appear anywhere in the output.
        assert!(
            !out.contains("\\n"),
            "labels must use no literal `\\n` (v11 breaks on it):\n{out}",
        );

        // classDefs are present so the marker classes resolve.
        assert!(out.contains("classDef focus"), "{out}");
        assert!(out.contains("classDef boundary"), "{out}");
    }

    #[test]
    fn caps_nodes_and_reports_true_total() {
        // A star: focus with five callees. depth 1 collects focus + 5 = 6 nodes.
        let mut symbols = vec![sym("k::focus")];
        let mut edges = Vec::new();
        for i in 0..5 {
            let callee = format!("k::callee{i}");
            symbols.push(sym(&callee));
            edges.push(edge("k::focus", &callee, EdgeKind::Static));
        }
        let idx = index(symbols, edges);

        let env = render_neighborhood(&idx, id("k::focus"), 1, 3);
        assert_eq!(env.total, 6, "true neighborhood size reported even when capped");
        assert!(env.truncated, "cap of 3 against 6 nodes must set truncated");
        assert!(
            env.data.contains("trunc_note"),
            "a truncation note must be drawn when capped:\n{}",
            env.data,
        );
        // Only the capped number of workspace symbols are drawn (plus the note).
        let drawn = declared_ids(&env.data);
        let symbol_nodes = drawn.iter().filter(|i| i.starts_with('n')).count();
        assert_eq!(symbol_nodes, 3, "exactly node_limit workspace symbols drawn");
    }

    #[test]
    fn depth_bounds_the_neighborhood() {
        // a -> b -> c -> d, focus at a. depth 1 sees {a, b}; depth 2 sees {a,b,c}.
        let idx = index(
            vec![sym("k::a"), sym("k::b"), sym("k::c"), sym("k::d")],
            vec![
                edge("k::a", "k::b", EdgeKind::Static),
                edge("k::b", "k::c", EdgeKind::Static),
                edge("k::c", "k::d", EdgeKind::Static),
            ],
        );
        assert_eq!(render_neighborhood(&idx, id("k::a"), 1, 50).total, 2);
        assert_eq!(render_neighborhood(&idx, id("k::a"), 2, 50).total, 3);
        assert_eq!(render_neighborhood(&idx, id("k::a"), 3, 50).total, 4);
    }

    #[test]
    fn generic_path_label_is_escaped_not_a_bare_id() {
        // A path with `<`/`>` must stay inside a quoted, escaped label.
        let idx = index(
            vec![sym("k::focus"), sym("k::wrap::<Vec<String>>")],
            vec![edge("k::focus", "k::wrap::<Vec<String>>", EdgeKind::Static)],
        );
        let out = render_neighborhood(&idx, id("k::focus"), 1, 50).data;
        assert!(out.contains("&lt;Vec&lt;String&gt;&gt;"), "angle brackets escaped:\n{out}");
        assert!(!out.contains("::<Vec"), "raw generic path must not leak into an id:\n{out}");
    }

    #[test]
    fn missing_focus_yields_valid_empty_flowchart() {
        let idx = index(vec![sym("k::a")], vec![]);
        let env = render_neighborhood(&idx, id("k::does_not_exist"), 2, 50);
        assert!(env.data.contains("flowchart"));
        assert_eq!(env.total, 0);
        assert!(env.data.contains("not found"));
    }
}
