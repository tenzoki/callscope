# Code Review — Turn 2: callscope-index (P4) + callscope-mcp (P7)

**Sender:** coderev
**Date:** 260726-2210
**Scope:** the two crates committed this Turn — `crates/callscope-index/**`
(commit ec14c5b) and `crates/callscope-mcp/**` (commit ccdace0). `callscope-core/src/query.rs`
(dbf0f50, CR1) glanced at only to confirm the earlier fix is coherent.

## Summary

Both crates are well-built and the design is sound. The fail-on-collision guard
is real and correctly closes the earlier silent-merge issue. The MCP server wires
staleness on every request and preserves every envelope flag through its mirror
remap. The substantive findings are all about *generality beyond the fixture*:
the `dyn` over-approximation — the tool's honesty centrepiece — silently drops
implementors in two real-workspace shapes (generic implementors, cross-crate
implementors), and the `boundary_applies` flag over-triggers on ordinary std
calls. None blocks the P11 acceptance harness except the boundary-flag semantics,
which P11 must account for on forward walks.

## Totals

- Critical: 0
- High: 0
- Medium: 4 (2 issues + 2 decisions)
- Low: 2 (issues)

Filed as 4 issues + 2 decision records under this Circle. Plus one non-filed
deferred item (unused imports) and confirmations below.

## Verification performed

- `cargo test -p callscope-core` -> 46 passed, 0 failed.
- `cargo test -p callscope-mcp` -> handlers integration test 5 passed, 0 failed.
- `cargo check -p callscope-index` -> Finished clean, **zero warnings**.
- `cargo build -p callscope-mcp --tests` -> surfaced the two deferred unused-import
  warnings, located precisely (below).
- Source read of all Turn-2 files + the core types the server mirrors.
- The two dyn-dispatch findings are **inference** (read from source, not run):
  building the nightly compiler-linked crate against a generic/cross-crate
  implementor fixture was out of scope, so I did not reproduce them empirically.

## Findings

### dyn over-approximation — the Q1/Q2 centrepiece (both real-workspace gaps)

**1. [Medium, issue] Generic implementors dropped.** `graph_build.rs:299` —
`Instance::resolve(impl_fn, &GenericArgs(vec![]))` resolves impl methods with no
generic arguments, so a generic implementor (`impl<T> Tokenizer for Wrapper<T>`)
or a generic trait method resolves `Err` and its virtual edge is never emitted.
The widened set silently misses an implementor that could fire — the Q2-forbidden
silent under-approximation, and it contradicts README:117-121 ("none that could
fire is missing"). Fixture uses non-generic `Simple`/`Fancy`, so P11 is unaffected.
Filed: `issues/260726-2210_o_emit-virtual-empty-genericargs-drops-generic-implementors.md`.

**2. [Medium, issue] Cross-crate implementors missed.** `graph_build.rs:288` —
`local_crate().trait_impls()` finds only implementors defined in the same crate
as the `dyn` call site. In a multi-member workspace where implementors live in a
different member, no fragment ever emits the edge, and `merge` does not synthesize
it. Fixture keeps trait + implementors + use-site in one `parser` crate, so P11 is
unaffected. Filed: `issues/260726-2210_o_emit-virtual-only-enumerates-local-crate-impls.md`.

Together these mean the "every workspace implementor" claim currently holds only
for non-generic, same-crate implementors. Recommend softening the README until
both are fixed.

### Boundary semantics — signal quality + the one P11-relevant item

**3. [Medium, decision] `boundary_applies` fires on ordinary std calls.**
`graph_build.rs:247-260` emits a call edge for every `Item` callee regardless of
crate (locality gates only *walking*, not edge emission); `query.rs:180-185`
marks any absent target a boundary. So every forward walk (C2 callees, C3 forward,
C6) over real code sets `boundary_applies` because functions call std — a flag
true almost always loses the signal §7 wants (a chain continuing *through* a
dependency). Backward walks (affected_tests, impact) are unaffected — verified:
`note_edge` reads `edge.to`, which on an incoming edge is the workspace node.
**This is the one finding that touches P11**: any assertion that a forward answer
has `boundary_applies == false` will fail against the real fixture if the walked
functions call std. Decide the boundary scope (or shape P11's assertions) first.
Filed: `decisions/260726-2210_o_boundary-applies-triggered-by-std-calls-on-forward-walks.md`.

### MCP server behavior

**4. [Medium, decision] Server hard-fails at start-up with no index.**
`main.rs:54` propagates `IndexState::load` error out of `main`, so on an
unindexed workspace the auto-launched plugin exits before serving and no tool
registers — the agent gets no in-band "run callscope-index" signal. Recommend
starting unconditionally and returning a no-index error envelope per tool. Filed:
`decisions/260726-2210_o_mcp-server-behavior-when-no-index-present.md`.

**5. [Low, issue] Mirror structs duplicate core query payloads.** `state.rs:43-63`
mirrors `DirectCalls`/`CallPath`/`Impact` because core does not derive `Serialize`.
Currently safe (exhaustive destructuring catches new fields at compile time; the
remap copies all six envelope flags), but redundant. Fix: derive `Serialize` on
the core payloads and delete the mirrors. Filed:
`issues/260726-2210_o_mcp-mirror-structs-duplicate-core-query-payloads.md`.

**6. [Low, issue] Staleness re-hashes the whole workspace on every request.**
`state.rs:156-163` via hash-all fingerprint, called by all eight tools. Correct
but repeated per tool call; the plan wanted mtime-first. Deferrable performance.
Filed: `issues/260726-2210_o_staleness-rehashes-whole-workspace-every-request.md`.

## What was checked and found sound

- **fail-on-collision is real.** `main.rs::merge:183-236` dedups by `SymbolId`,
  detects a same-id/different-path clash, and aborts with a diagnostic naming both
  paths. The Builder keys its per-fragment map by `fq_path` (`graph_build.rs:73`),
  so both colliding symbols survive into the fragment for `merge` to catch — the
  gap that made the earlier issue "silent" is genuinely closed. Two unit tests
  cover collision-aborts and same-symbol-unions. Since the indexer aborts before
  writing, `Graph::new`'s last-write-wins `by_id` can never see a collision.
  Closes `issues/260726-1930_c_symbolid-collision-...`.
- **Staleness on every request (Q6).** All eight tools call `self.stale()?` before
  answering and attach the result at the single serialization point
  (`state.rs::envelope_to_json`). Correct.
- **Envelope flag preservation.** `remap_envelope` (`state.rs:69-78`) copies all
  six fields; payload conversion uses exhaustive struct patterns, so no flag and
  no field is silently dropped. Verified against `envelope.rs:66-83`.
- **Ambiguous-symbol handling (Q3).** `resolve_one` precedence numeric-id ->
  exact-path -> fragment search; ambiguous or not-found both return the candidate
  envelope, never a guess. `call_paths_by_name` short-circuits per endpoint.
- **Scratch wipe.** `orchestrate` wipes only `.callscope/build`, not `.callscope`,
  so the previous `index.bin`/`manifest.json` are never destroyed before the new
  ones are written. The cold-rebuild rationale is documented and correct for
  determinism. (Minor: a private `CARGO_TARGET_DIR` under the wiped scratch means
  third-party deps recompile cold each run too — a Q5 cost, folded into finding 6's
  performance theme, not separately filed.)
- **Panics on compiler data.** The earlier `FnDef::fn_sig().unwrap()` panic is
  fixed via `fn_sig_is_unsafe` (`graph_build.rs:419-426`), used in both call sites.
  `handle_call` reads `func.ty` / `fn_def` through `Result`/`Option` guards;
  `body_of_def` returns `None` for async-closure defs rather than panicking;
  `collect_closures` guards recursion with a `seen` set. No new unguarded
  `.unwrap()` on compiler data found in the Turn-2 paths.
- **CR1 fix (query.rs call_paths) is coherent.** Collects `max_paths+1`, sets
  `hit_cap = len > max_paths`, and accumulates virtual/boundary flags during the
  DFS (not off survivors). The three regression tests assert the fixed behavior.
  The two related issues are marked closed. Confirmed.

## Deferred item (not filed, as instructed)

The two `unused import` warnings are **`crates/callscope-mcp/tests/handlers.rs:12`
(`std::collections::BTreeMap`) and `:18` (`Manifest`)** — not in callscope-index
as anticipated. `cargo check -p callscope-index` is warning-free. (The same test
compile also emits several `dead_code` warnings for `state.rs` items — `Resolved`,
`call_paths_by_name`, the `DEFAULT_*` consts, etc.; these are artifacts of the
test including `state.rs` via `#[path]` without `tools.rs`, and those items are
used in the real binary. Not defects.)

## Recommended sequencing

- **Before P11:** resolve the boundary-flag scope (finding 3) or write P11 so it
  never asserts `boundary_applies == false` on a forward walk — otherwise the
  harness asserts against a flag that is true almost everywhere.
- **Before real-workspace use (post-acceptance):** fix the two dyn gaps
  (findings 1, 2); they are the difference between the honest over-approximation
  the tool promises and a silent miss. Soften the README claim until then.
- **Cleanup, any time:** findings 4 (server UX), 5 (mirror consolidation),
  6 (fingerprint perf).

Nothing here blocks Turn-2 closure. Only finding 3 constrains how the P11
acceptance harness is written.
