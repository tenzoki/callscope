# FIX-DYN — close the two silent dyn-dispatch under-approximation gaps

**Session:** 260726-2253
**Agent:** coder
**Status:** Complete

## Task

Close the two Turn-2 review gaps where the `dyn Trait` over-approximation
silently missed implementors (tender Q2: "a silent best guess is a defect"):

- generic implementors (`impl<T> Tokenizer for Wrapper<T>`) dropped, and
- implementors defined in a different workspace member than the `dyn` call site.

Coupled fixture-extension + indexer fix, scope `fixtures/workspace/**` and
`crates/callscope-index/**` only.

## What I changed

### Part A — fixture (`fixtures/workspace/**`)
- `parser/src/lib.rs`: added a generic implementor `Wrapper<T>` (`impl<T:
  Tokenizer> Tokenizer for Wrapper<T>`) whose `tokenize` reaches
  `normalize_token`, plus a unit test `reaches_via_dyn_wrapper` that drives
  `Wrapper<Simple>` through `run_dyn`'s `&dyn Tokenizer` site. Updated the
  module reachability map.
- New member crate `ext_tokenizer/` (wired into `fixtures/workspace/Cargo.toml`
  `members`): defines `Shouty`, a cross-crate implementor of `parser::Tokenizer`
  reaching `parser::normalize_token`, with a test `cross_crate_reaches_via_dyn`
  driving it through `parser::run_dyn`.
- Existing constructs and the guiding example left intact. `cargo test` in the
  fixture: parser 5 unit + 2 integration, ext_tokenizer 1 — all pass.

### Part B — indexer (`crates/callscope-index/**`)
Root cause of both gaps: `emit_virtual` resolved implementors per crate from
`local_crate().trait_impls()` and resolved impl methods with empty
`GenericArgs`. On this nightly the empty-args resolve of a generic impl returned
a *polymorphic* instance whose `body()` PANICS — so the generic case aborted the
whole compilation, not merely dropped an edge.

Redesign — move implementor enumeration to merge time:
- `graph_build.rs`: `Fragment` gains `dyn_calls: Vec<DynCall>` and
  `impl_methods: Vec<ImplMethodFact>`. `record_dyn_call` records each
  `dyn`/unresolved-trait call site (owner, trait path, method) instead of
  enumerating locally. `collect_impls` inventories every local trait impl; for a
  generic impl it walks the polymorphic body via `FnDef::body()` (item-level
  `mir_body`, no monomorphization, no panic) into a `generic`-flagged symbol.
- `main.rs::merge`: joins every `DynCall` against every crate's
  `ImplMethodFact`s and emits one `Virtual` edge per implementor — so the
  widened set spans ALL workspace members and includes generic implementors.
  Also sorts edges for a byte-deterministic index (edges were never sorted;
  pre-existing, and my HashSets would have added ordering).
- New unit test `merge_joins_dyn_calls_to_all_workspace_implementors`.

Honesty: `over_approximated`/`implementor_count` are unchanged in the query
layer (not in scope); they are derived from distinct virtual-edge targets, so
the count is honest by construction. The generic implementor is represented by a
single polymorphic node (flagged `generic`) — a documented residual, not an
exact claim.

## Verification (end-to-end)

Rebuilt indexer, ran `callscope-index fixtures/workspace`, loaded the produced
index into `callscope-core` via a throwaway harness running `Graph`:

| Query | Before | After |
|---|---|---|
| `run_dyn` dyn implementors | 2 (Simple, Fancy) | 4 (Simple, Fancy, `Wrapper<T>` generic, `ext_tokenizer::Shouty` cross-crate) |
| `over_approximated.implementor_count` | 2 | 4 |
| `affected_tests(normalize_token)` | 6 tests | 8 tests |

Guiding example holds: `affected_tests(normalize_token)` still includes both
generic-dispatch tests (`reaches_via_generic_dispatch`,
`integration_reaches_via_generic`); the two new dyn paths
(`reaches_via_dyn_wrapper`, `cross_crate_reaches_via_dyn`) now correctly appear.

Other checks: `cargo test` whole callscope workspace 46+3 pass, 0 failures;
`cargo check -p callscope-index` 0 warnings; index byte-deterministic across two
runs (same sha).

## Outcome

- Issue `260726-2210_..._only-enumerates-local-crate-impls` — FULLY closed
  (renamed `_c_`).
- Issue `260726-2210_..._empty-genericargs-drops-generic-implementors` — closed;
  generic implementor now walked and included, residual documented (renamed
  `_c_`).
- Decision `260726-2253_a_generic-implementor-over-approximation-representation.md`
  filed documenting the single-polymorphic-node representation residual.

Did not touch callscope-core, callscope-mcp, the plan, or README (out of scope).
Not committed — orchestrator commits.
