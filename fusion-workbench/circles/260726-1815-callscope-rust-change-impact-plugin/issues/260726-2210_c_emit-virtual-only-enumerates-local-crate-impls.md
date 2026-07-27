dyn over-approximation misses implementors defined in a different workspace crate than the call site
---
`emit_virtual` enumerates trait implementors from `local_crate().trait_impls()`
only: `crates/callscope-index/src/graph_build.rs:288`. `local_crate()` is the
crate currently being compiled by the wrapper. So a `&dyn Trait` call site in
workspace member crate A is over-approximated only to the implementors of that
trait that are **also defined in A**. An implementor defined in workspace member
crate B is not found at A's call site, and B's own compilation never sees A's
call site, so no virtual edge A-owner -> B-impl is ever emitted by either
fragment. The `merge` step only deduplicates symbols and edges; it does not
synthesize cross-crate virtual edges.
---
Severity: Medium. Calibration: inference — read from source, not run.

Why it matters: the README (`README.md:117-121`) and the tool's premise promise
over-approximation to "every workspace implementor" of the trait. In a
multi-crate workspace where implementors live in a different member than the
`dyn` call site, the widened set is incomplete — an under-approximation, the
Q2-forbidden silent miss, not the honest superset.

Does NOT block P11 acceptance: the fixture keeps the `Tokenizer` trait, its
`Simple`/`Fancy` implementors, and the `&dyn Tokenizer` use site all inside the
single `parser` crate, so the local-only enumeration is complete there. This is
a real-workspace generality gap.

Fix direction: enumerate trait impls across all workspace-member crates, not just
`local_crate()`. Two shapes are possible: (a) collect trait-impl -> implementor
facts per crate into the fragment and resolve virtual edges in the orchestrator's
`merge`, where all crates' facts are present; or (b) have each crate emit its own
trait-impl inventory and join dyn call sites to it at merge time. Either moves
the implementor lookup to where the whole workspace is visible.

Affects: callscope-index (P4), `graph_build.rs::emit_virtual` + `main.rs::merge`.

---
Resolved: FULLY closed. The per-crate `emit_virtual` (which enumerated only
`local_crate().trait_impls()`) is replaced by a two-part merge-time join. Each
crate now records (a) its `&dyn Trait` / unresolved-trait call sites as
`DynCall` facts and (b) its trait-implementor inventory as `ImplMethodFact`s
(`crates/callscope-index/src/graph_build.rs`: `record_dyn_call`, `collect_impls`).
The orchestrator's `merge` (`crates/callscope-index/src/main.rs`) joins every
`DynCall` against every crate's `ImplMethodFact`s and emits a `Virtual` edge per
implementor — so a `&dyn Tokenizer` site in `parser` now widens to
`ext_tokenizer::Shouty`, an implementor defined in a DIFFERENT workspace member.
Verified end-to-end: `parser::run_dyn`'s over-approximation went from 2
implementors to 4 (adds cross-crate `Shouty` and generic `Wrapper`), and
`affected_tests(parser::normalize_token)` now includes
`ext_tokenizer::tests::cross_crate_reaches_via_dyn`. New unit test
`merge_joins_dyn_calls_to_all_workspace_implementors` covers the cross-crate
join. Fixture extended with the `ext_tokenizer` member crate.
