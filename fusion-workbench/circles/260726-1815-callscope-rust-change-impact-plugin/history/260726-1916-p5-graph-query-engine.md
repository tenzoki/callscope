# P5 — Graph query engine in callscope-core

**Date:** 2026-07-26
**Agent:** coder
**Status:** Complete
**Task:** P5 (tasklist), plan step 5
**Scope:** `crates/callscope-core/src/query.rs` only.

## What was implemented

Filled the comment-only `query.rs` stub with the pure graph-query algorithms
behind capabilities C1–C7. No dependencies added, no other module or manifest
touched. All work is pure functions over a loaded `Index` (schema.rs), returning
`Envelope<T>` (envelope.rs) directly so answers carry their own uncertainty.

### API

A `Graph<'a>` view is built once from `&Index` (`Graph::new`), precomputing the
id lookup plus forward/backward adjacency lists sorted by neighbour `fq_path`
(the source of deterministic ordering). Query methods:

| Cap | Method | Returns |
|-----|--------|---------|
| C1 | `resolve(query, limit)` | `Envelope<Vec<Symbol>>` — candidate set, case-insensitive substring on fq_path, never picks one (Q3) |
| C2 | `direct_calls(id, limit)` | `Envelope<DirectCalls>` — `{ callers, callees }` (1-hop) |
| C3 | `reachability(id, dir, limit)` | `Envelope<Vec<Symbol>>` — transitive, `Direction::{Forward,Backward}` |
| C4 | `call_paths(from, to, max_depth, max_paths)` | `Envelope<Vec<CallPath>>` — simple paths, bounded |
| C5 | `affected_tests(id, limit)` | `Envelope<Vec<Symbol>>` — backward reach filtered to `test` |
| C6 | `reachable_unsafe(id, limit)` | `Envelope<Vec<Symbol>>` — forward reach filtered to `uses_unsafe` |
| C7 | `impact(id, limit)` | `Envelope<Impact>` — `{ direct_callers, affected_tests }` merged |

Supporting public types: `Direction`, `DirectCalls`, `CallPath`, `Impact`, and
the constant `DYN_TRAIT_MARKER`.

### Flag propagation (real, not TODO)

- **Over-approximation (Q2):** an internal `WalkFlags` accumulator sets
  `over_approximated` whenever a walk crosses an `EdgeKind::Virtual` edge. It is
  stamped onto the envelope as `Reason::DynDispatch { trait_path, implementor_count }`
  where `implementor_count` is the number of distinct virtual callees folded in.
  `trait_path` is the generic marker `DYN_TRAIT_MARKER` (`"<dyn dispatch>"`)
  because v1's `Edge` carries no dispatched-trait identity — noted in the module
  doc so P4 can enrich it without changing this contract.
- **Boundary:** since v1's schema has no boundary edge kind, a boundary crossing
  is represented structurally: an edge whose target is absent from the symbol
  table (third-party crate filtered out) or present but `characteristics.foreign`.
  Either flips `boundary_applies`; such a target is a walk leaf (not descended
  into).
- **Bounded output (Q4):** every set-returning query walks the full finite set
  first (cycle-safe), reports the true `total`, then sorts and truncates to
  `limit`, setting `truncated` when the limit cut the answer.
- **Staleness (Q6):** deliberately never set here — it is index-vs-disk, which
  the query layer cannot see. `callscope-mcp` (P7) attaches it via
  `Envelope::with_stale`.

### Algorithm notes

- Reachability excludes the start symbol (appears only via a cycle back to it).
- Cycles are handled with a visited/enqueued set (reachability BFS) and a
  simple-path constraint (path DFS), so neither loops forever.
- `call_paths` sets `truncated` when the `max_paths` cap is hit OR a branch was
  cut at `max_depth` before reaching the target (a longer path may be missed).
- Determinism: adjacency sorted by neighbour fq_path; all result vectors sorted
  by `(fq_path, id)`; paths sorted by their fq_path sequence.

## Verification

Command: `cargo test -p callscope-core`
Result: **38 passed, 0 failed** (24 new query tests + P2's 7 + P3's fingerprint
tests). `cargo build -p callscope-core --all-targets` is warning-free.

Test coverage includes: reachability both directions; a virtual edge flips
over-approximation (and static-only does not); foreign-target and absent-target
boundary flips; path enumeration between two nodes; path count-cap and depth-cap
truncation; affected-tests picks only test symbols including one reachable only
through a virtual edge (the guiding-example shape); reachable-unsafe filters;
cycle safety for both reachability and path enumeration; truncation reports the
full total; impact combines callers + tests and merges flags from both halves.

(clippy not run — not installed in the pinned nightly's minimal profile; out of
scope to add a component.)

## Tracking updated

- tasklist P5 → `[x]`
- plan step 5 → `[DONE]`

## Files changed

- `crates/callscope-core/src/query.rs` (implemented; was a comment stub)
