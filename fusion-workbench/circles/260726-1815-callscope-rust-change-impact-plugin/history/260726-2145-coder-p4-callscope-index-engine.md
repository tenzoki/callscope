# P4 — callscope-index rustc-driver indexing engine (resume + finish)

**Agent:** coder
**Status:** Complete
**Date:** 2026-07-26
**Scope touched:** `crates/callscope-index/**` only (main.rs, graph_build.rs). No changes to callscope-core, callscope-mcp, query.rs, or fixture source.

## What was done

Resumed from an interrupted session: the driver was on disk and `cargo build`
was green, but no index had been produced. Ran the indexer end-to-end against
`fixtures/workspace/`, iterated on every failure, and produced a working
`index.bin` + `manifest.json` that loads into `callscope-core`/`callscope-mcp`.

### Bugs found and fixed (all in crates/callscope-index)

1. **Panic: `FnDef::fn_sig().unwrap()` on `None` (graph_build.rs).** A local
   closure-call **shim** (`InstanceKind::Shim`) was enqueued as a callee; its def
   type is a closure type, not a fn type, so `fn_sig()` unwrapped `None` and
   aborted the whole compilation. Fix: only walk / symbolise `InstanceKind::Item`
   instances (shims, intrinsics, vtable-virtual are neither symbols nor edge
   targets), and read signatures through a non-panicking `fn_sig_is_unsafe`
   helper (`inst`/`fndef` `.ty().kind().fn_sig()` → `Option`).

2. **Incomplete index from stale build cache (main.rs).** The orchestrator
   deleted old fragments but reused `target/`, so on any re-index cargo served
   cached workspace members, their wrappers never re-ran, no fragments were
   emitted, and those crates silently vanished from the graph (a Q1 silent-wrong
   answer). Fix: wipe the whole `.callscope/build` scratch each run so every
   member recompiles through the wrapper. (Whether to re-index at all is P3's
   staleness job, upstream.)

3. **`test` characteristic always false → C5 empty (graph_build.rs).** Detection
   read `#[rustc_test_marker]` off the fn, but the harness puts that marker on a
   generated `const`, and in this nightly it is a *parsed* builtin attribute that
   `get_attrs` no longer surfaces by name. Fix: harvest the harness consts
   directly — a `Const` whose type is the `test::TestDescAndFn` ADT, carrying the
   *same fq_path as the test fn* (impossible for ordinary source); a fn is a test
   iff its path matches a harvested const.

4. **Closure-body calls not folded → generic-dispatch reachability lost
   (graph_build.rs, gap 3).** `Simple::tokenize` reaches `normalize_token` only
   through a non-capturing `.map(|t| normalize_token(t))` closure. A
   non-capturing closure is a ZST: its construction never appears as an
   `Aggregate(Closure)` rvalue, and it is not a named local either — it survives
   only nested inside the adapter type `std::iter::Map<_, {closure}>`. The old
   construction-site scan missed it, so `Simple::tokenize → normalize_token` was
   absent and both generic-dispatch tests dropped out of C5. Fix: fold closures
   by a **recursive search of every local's type tree** (`collect_closures`),
   guarded by a `HashSet<Ty>`; catches capturing and non-capturing closures and
   attributes them to the concrete owner instance.

5. **fail-on-collision completeness (main.rs + graph_build.rs).** `merge()`
   already failed on cross-fragment `SymbolId` collisions, but the per-crate
   `Builder` deduped symbols **by id**, so a within-crate collision was dropped
   before `merge` could see it. Fix: key the Builder symbol table by `fq_path`
   so both survive into the fragment and reach the check. Added two unit tests.
   Closes issue `260726-1930_*_symbolid-collision-has-no-detection-silent-merge`.

## On-disk format (decision 260726-2108)

Confirmed compliant: `index.bin` and `manifest.json` are both **serde_json** of
the verbatim `callscope-core::schema` `Index` / `Manifest` types
(`serde_json::to_vec` / `to_vec_pretty`). Authoritatively verified by loading
both with `serde_json::from_slice` into the core types (the same path
`callscope-mcp` uses) — LOADED OK.

## Verification

`cargo build -p callscope-index` clean (no warnings). `cargo test -p
callscope-index`: 2 passed (collision + union). Ran the indexer on a fresh clean
workspace and loaded the output through the **real** `callscope-core` query
functions in a throwaway harness:

- Index: **16 symbols, 66 edges**, schema_version 1, toolchain
  nightly-2026-07-26-aarch64-apple-darwin.
- `normalize_token` present as a symbol (`resolve` → 1 hit).
- **C5 `affected_tests(normalize_token)` total=6**, includes BOTH the in-crate
  `#[test] reaches_via_generic_dispatch` and the separate-crate
  `integration::integration_reaches_via_generic` — the load-bearing
  generic-dispatch cases — plus the two dyn cases, the direct case, and the
  `#[tokio::test]` async case. Envelope `over_approximated =
  DynDispatch { implementor_count: 2 }` (Q2 flag propagates through the backward
  walk).
- **Q2:** the `run_dyn` `dyn` call site produces two `Virtual` edges
  (→ Simple::tokenize, → Fancy::tokenize).
- **C6 `reachable_unsafe(normalize_token)`** → `ensure_round_trip`.
- Gap 1 (monomorphization): `run_generic::<parser::Simple>` present as a
  specialised symbol with a `Static` edge to `Simple::tokenize`.

All guiding-example facts present. No gaps had to be faked on this nightly's
`rustc_public` API.

## Tracking

- Plan step 4 → `[DONE]`.
- tasklist P4 → `[x]`.
- Issue `260726-1930_o_symbolid-collision…` → `_c_` (Resolved note appended).
- Decision `260726-2108_a_on-disk-index-serialization-format`: the writer now
  realises the recorded answer (serde_json, core types). Left as `_a_` — the
  `Implemented:` line needs the commit hash, which the orchestrator produces;
  recommend it flip `_a_`→`_i_` with the hash at commit time.

## Note for P11 (acceptance)

The index is produced and loads through `callscope-core`. P11 should run the
authoritative acceptance against `callscope-mcp` loading this
`fixtures/workspace/.callscope/index.bin`.
