# P11 — acceptance harness (C1–C8 + Q1–Q6 against the fixture)

**Agent:** coder
**Status:** Complete
**Date:** 2026-07-26
**Scope touched:** added `crates/callscope-mcp/tests/acceptance.rs` only. No
production code in callscope-core / callscope-index / callscope-mcp changed.
Tracking: plan step 11 → `[DONE]`, tasklist P11 → `[x]`.

## What was built

An automated acceptance harness — the formal proof the Directive is met. It is a
single `#[test]` in `crates/callscope-mcp/tests/acceptance.rs` that:

1. Runs the **real** `callscope-index` binary against `fixtures/workspace/`
   (twice, for the Q5 determinism check). `.callscope/` is gitignored, so the
   harness produces ground truth itself rather than loading a committed index.
2. Loads the produced `index.bin` + `manifest.json` through the **authoritative
   answer path** — `callscope-mcp`'s `IndexState`, the same handler layer the MCP
   tools call (pulled in via `#[path = "../src/state.rs"]`, the pattern
   `tests/handlers.rs` already uses).
3. Asserts 19 checks, each returning `Ok(evidence)` / `Err(reason)` and printed
   as a labelled PASS/FAIL breakdown so one failure never hides the rest.

The harness links no compiler internals — it shells out to the prebuilt indexer
binary. It does NOT nest a `cargo build` (that would deadlock on the workspace
target lock); it locates `target/{debug,release}/callscope-index` (or
`$CALLSCOPE_INDEX_BIN`) and fails with the build command if absent.

## How to run

```
cargo build -p callscope-index
cargo test  -p callscope-mcp --test acceptance -- --nocapture
```

The whole workspace is pinned to the nightly via `rust-toolchain.toml`, so one
toolchain serves both the indexer build and the test run. Runtime ~7s (two cold
fixture index builds dominate).

## Result — 19/19 PASS

Verified: `cargo test -p callscope-mcp` → acceptance 1 passed, handlers 5 passed,
0 failed. Fixture index: 21 symbols, 93 edges, deterministic across two runs.

| Check | Capability / requirement | Evidence |
|---|---|---|
| C1 | resolve → symbol + characteristics | `resolve("normalize_token")` → `parser::normalize_token` (public=true) |
| C2 | direct callers + callees | callers = 5 (Simple/Fancy/Wrapper/Shouty tokenize + normalizes_directly); callees = ensure_round_trip |
| C3 | reachability both ways | forward(run_generic)→3 incl normalize_token+ensure_round_trip; backward(normalize_token)→16 incl run_dyn+run_generic |
| C4 | call_paths | `run_generic::<Simple> → Simple::tokenize → normalize_token`, via BOTH the by-name tool path and the by-id helper |
| C5 | affected_tests (load-bearing §6) | 8 tests incl BOTH `reaches_via_generic_dispatch` (in-crate) AND `integration_reaches_via_generic` (separate crate); over_approximated |
| C6 | reachable_unsafe | `parser::ensure_round_trip` |
| C7 | impact = callers + tests | 5 callers + 8 affected tests, total 13 |
| C8 | neighborhood Mermaid | flowchart, safe generated ids, no literal `\n`, classDefs present |
| DYN | FIX-DYN four-implementor coverage | run_dyn widens to Simple, Fancy, `Wrapper<T>` (generic), `ext_tokenizer::Shouty` (cross-crate); implementor_count=4 |
| Q1-gen | gap 1 monomorphization | `run_generic::<Simple>` present (generic), statically calls Simple::tokenize |
| Q1-dyn | gap 2 dyn dispatch | run_dyn → 4 Virtual edges |
| Q1-body | gap 3 closure + async bodies | `.map(\|t\| normalize_token)` folded into Simple::tokenize; async body folded into tokenize_async |
| Q1-test | gap 4 macro tests | both `#[test]` and `#[tokio::test]` carry `test` |
| Q2 | over-approximation flagged | affected_tests flagged `DynDispatch { implementor_count: 4 }` |
| Q3 | ambiguous → candidates | `"tokenize"` → 8 candidates, `resolve_then` returns the candidate set, no silent pick |
| Q4 | capped list reports total | limit=3 → returned 3, total=8, truncated=true |
| Q5 | repeatable indexing | two runs byte-identical index.bin (12546 bytes) |
| Q6 | staleness detectable | temp copy not stale; after editing `parser/src/lib.rs`, stale with `diverged_files=["parser/src/lib.rs"]` |
| BND | boundary flag set on third-party crossing | forward walk from ensure_round_trip (calls std) sets `boundary_applies=true` |

## Honest notes on the guidance in the task

- **C4 is genuinely served, not dead.** The Turn-2 hint suggested some
  `call_paths` helper code might be dead. Traced the wiring: MCP `call_paths`
  tool → `call_paths_by_name` → `IndexState::call_paths` (by id) →
  `Graph::call_paths`. Nothing is dead. The harness confirms empirically by
  exercising BOTH the by-name tool path and the by-id helper — both return the
  real path. No issue filed (there is no defect).
- **Boundary flag** — followed the Turn-2 review: did NOT assert
  `boundary_applies == false` on a forward answer (it fires on ordinary std
  calls). Asserted it is *set* where a third-party edge is genuinely crossed
  (forward from `ensure_round_trip`, which calls std). Meaningful, not a false
  claim of exactness.
- **Q6** runs in an isolated temp copy of the fixture (fixture copied minus
  `target/` and `.callscope/`, then the produced index artifacts copied in), so
  the committed fixture is never dirtied. The manifest's workspace-relative file
  hashes match the faithful copy, so a clean load is correctly not-stale.
- Q1's `implementor_count=4` and the dyn coverage confirm FIX-DYN's enrichment
  end-to-end (generic `Wrapper<T>` and cross-crate `Shouty` both included).

## Verdict

Every capability C1–C8 and every quality requirement Q1–Q6 is **demonstrably
proven** against the fixture, including the load-bearing §6 answer. None had to
be weakened to pass. No defect surfaced; no issue filed. No production behavior
changed. Not committed — orchestrator commits.

All 11 plan steps are now `[DONE]`; the plan is ready for Circle-closure review
by the orchestrator.
