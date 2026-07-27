# P6 — C8 Mermaid neighborhood renderer in callscope-core

**Date:** 2026-07-26
**Agent:** coder
**Status:** Complete
**Task:** P6 (tasklist), plan step 6, capability C8
**Scope:** `crates/callscope-core/src/mermaid.rs` only.

## What was implemented

Filled the comment-only `mermaid.rs` stub with the C8 neighborhood renderer: a
pure function that produces Mermaid `flowchart` text for the bounded call-graph
neighborhood around a focus symbol. No dependencies added, no Cargo.toml edit,
no other module touched (`pub mod mermaid;` was already declared in lib.rs).

### API

```
pub fn render_neighborhood(index: &Index, focus: SymbolId, depth: usize, node_limit: usize) -> Envelope<String>
```

Returns the flowchart text in the standard `Envelope<String>`.

### Neighborhood gathering (traversal reused, not reimplemented)

The neighborhood is every workspace symbol within `depth` **undirected** hops of
the focus (both callers and callees — a readable local graph). Each ring is
expanded by calling `Graph::direct_calls` (from query.rs) on the frontier nodes,
so this module leans on the existing adjacency and edge logic rather than
rewriting BFS. Boundary targets are not expanded (they are workspace-edge
leaves, matching query.rs's `is_boundary_target` rule: target absent from the
symbol table, or present but `foreign`). Rings are sorted by `fq_path` for a
deterministic layout and a deterministic cap.

## Rendering choices

- **Id scheme (v11 safety by construction):** node ids are always generated —
  `n<i>` for workspace symbols (BFS order, `n0` = focus), `b<i>` for synthetic
  boundary-target nodes, and `trunc_note` for the truncation marker. Raw
  `fq_path` never appears in an id position, so Mermaid reserved words
  (`graph`, `end`, `class`, …) and Rust-path punctuation (`::`, `<`, `>`, `(`,
  spaces) stay out of ids entirely. The human-readable path lives only inside
  the quoted `"label"`.
- **Edge styles:** Static = solid `-->`; Virtual (dyn) = dashed + labelled
  `-.->|dyn|`; boundary crossing = thick + labelled `==>|boundary|` (boundary
  styling takes precedence over kind).
- **Focus highlight:** the focus node carries `:::focus` (a `classDef` with a
  distinct fill/stroke). Boundary nodes carry `:::boundary`, the truncation note
  `:::trunc`. All three `classDef`s are emitted at the end.
- **Label escaping:** `&`, `<`, `>`, `"` are HTML-escaped so a generic path like
  `Vec<String>` cannot break the quoted label. No line breaks are ever inserted,
  so no label contains a literal `\n` (the v11 breakage guarded against).

### Envelope semantics

- **Q4 (bounded output):** the full neighborhood is collected first, so `total`
  is the true workspace-symbol count; `node_limit` caps the drawn symbols in BFS
  order (focus always survives a non-zero cap); when the cap cuts it, `truncated`
  is set and a `trunc_note` node ("+N more not shown (total M)") is drawn.
- **Q2 (over-approximation):** any drawn `EdgeKind::Virtual` edge sets
  `over_approximated = Reason::DynDispatch { trait_path: "<dyn dispatch>",
  implementor_count }`, mirroring query.rs's marker; `implementor_count` counts
  the distinct drawn virtual-edge targets.
- **Boundary:** a drawn edge leaving the workspace sets `boundary_applies`.
- **Missing focus:** a focus id that names no symbol yields a valid (near-empty)
  flowchart with a `%%` comment and `total = 0` — no panic.

## Verification

Command: `cargo test -p callscope-core`
Result: **43 passed, 0 failed** (5 new mermaid tests + the existing 38).

New tests assert: output contains `flowchart`; every declared node id is the
safe generated form and none is a reserved word; the focus path appears only
inside a quoted label with `:::focus`; a virtual edge renders as `-.->|dyn|` and
flips `over_approximated`; a boundary edge renders as `==>|boundary|` and flips
`boundary_applies`; no label contains a literal `\n`; the node cap reports the
true total and draws a truncation note; depth bounds the neighborhood
(depth 1/2/3 over an a→b→c→d chain give total 2/3/4); a generic path
`::<Vec<String>>` is escaped (`&lt;Vec&lt;String&gt;&gt;`) and never leaks into
an id; a missing focus yields a valid empty flowchart.

(No external mermaid CLI lint — the string-shape assertions are the gate, as the
task specifies. clippy not run — not in the pinned nightly's minimal profile.)

## Tracking updated

- tasklist P6 → `[x]`
- plan step 6 → `[DONE]`

## Files changed

- `crates/callscope-core/src/mermaid.rs` (implemented; was a comment stub)
