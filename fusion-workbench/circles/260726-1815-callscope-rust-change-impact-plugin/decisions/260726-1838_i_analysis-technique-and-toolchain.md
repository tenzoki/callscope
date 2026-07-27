# Which analysis technique gives compiler-grounded ground truth, and what toolchain cost does it impose?

---
**Domain:** code
**Status:** implemented
**Filed by:** planner
**Cross-references:** problem.md §2 (four gaps), §5 Q1 (ground truth), §7 (implementer's choice); circles/260726-1815-callscope-rust-change-impact-plugin/planning/260726-1838_o_callscope-implementation.md (the plan that depends on this answer)

---

## Question

callscope must answer change-impact questions from what the Rust compiler actually resolves, not from name matching over source (tender Q1). That requires closing four source-versus-execution gaps: generic instantiation and monomorphization, trait-object (`dyn`) dispatch, closure and `async fn` body attribution, and macro-expanded test items plus erased `unsafe`. The analysis technique is left to the implementer (tender §7), and the realistic techniques differ sharply in how many of the four gaps they close natively and in what they demand of the user's environment. The technique choice also fixes a toolchain cost that the user will carry on every workspace they index, so it should be ratified before the indexing engine is built.

## Options

1. **rustc custom driver over monomorphized MIR (via `rustc_public`, formerly `stable_mir`).** A small binary links the compiler as a library, runs a workspace crate through the monomorphization-collection stage, and reads the resulting mono-item call graph.
   - Pros: closes all four gaps with mechanisms the compiler already has. `rustc_monomorphize::collector` follows generic instantiations (gap 1), instantiates every `dyn`-compatible trait method on a trait-object cast (gap 2, the honest over-approximation Q2 asks to flag), and collects closure/`async`/drop-glue bodies as items that fold back to their parent function (gap 3). HIR attributes identify `#[test]`/`#[tokio::test]` and the unsafety check identifies `unsafe` usage (gap 4). `rustc_public` is the Rust project's public, SemVer-tracked API built for exactly this kind of external tool. Strong prior art: `cargo-call-stack`, nrc's `callgraph.rs`, Kani, MIRAI.
   - Cons: indexing requires a pinned nightly toolchain with the `rustc-dev` (and `llvm-tools-preview`) rustup components. `rustc_public` is public-but-not-frozen, so a toolchain bump can require a code update. Mono collection needs program roots, so each crate is compiled in test mode to seed roots.
2. **rust-analyzer as a library (`ra_ap_hir`, `ra_ap_ide`, `ra_ap_load-cargo`).** Build on rust-analyzer's HIR, type inference, and trait solver.
   - Pros: rich HIR, resolves method calls and trait implementors, "find references"/"call hierarchy" primitives exist; crates published to crates.io; IDE-grade robustness.
   - Cons: rust-analyzer does not perform monomorphization. Inside `run_generic<T: Tokenizer>` it resolves `tokenizer.tokenize()` to the trait method `Tokenizer::tokenize`, not to the concrete `Simple::tokenize` a caller instantiates — so gap 1, the tender's guiding example, is not closed without reimplementing instantiation propagation on top of RA. That reimplementation is the compiler's monomorphization collector, rebuilt by hand.
3. **LLVM-IR call metadata (`-Z call-metadata`) / cargo-call-stack extraction.** Read call edges (including dyn dispatch) that rustc can emit into LLVM IR.
   - Pros: dyn-dispatch edges and function-pointer edges are emitted by the compiler; no direct linkage against compiler internals.
   - Cons: still nightly-only and unstable; operates below MIR, so closure/async attribution to the user's source function and `#[test]`/`unsafe` source semantics are harder to recover than from MIR/HIR; parsing IR is a coarser, more brittle surface than the MIR API.

## Constraints

- Q1 is non-negotiable: the four gaps must all be closed, monomorphization included. That alone eliminates Option 2 as a standalone technique.
- The dependency boundary is drawn (settled Circle decision): analysis stops at the edge of the user's own workspace crates. The technique must let calls that cross into third-party crates be identified and marked, which a `DefId`-crate filter over the mono graph gives directly.
- The served answers and the agent workflow must run on a normal stable toolchain; only the indexing step may require a special toolchain.

## Recommendation

**Option 1 — rustc custom driver over monomorphized MIR via `rustc_public`.** It is the only technique that closes all four gaps with the compiler's own machinery rather than a partial re-implementation of it, and the `dyn`-dispatch over-approximation it produces is exactly the visible-uncertainty behavior Q2 requires. The cost to ratify is real and should be surfaced to the user plainly: **indexing a workspace requires installing a specific nightly toolchain with the `rustc-dev` component**, pinned via a `rust-toolchain.toml` that callscope ships. The cost is contained to the indexing step — the MCP server that serves answers and the agent-facing workflow need only stable Rust, because the server reads a pre-built index and never links the compiler.

If the user rejects the nightly requirement outright, there is no known path to compiler-grounded monomorphized reachability today, and the honest consequence is that Q1 cannot be met on a stable-only toolchain. That trade-off is the substance of this decision.

---
Answered:
Implemented:
Deferred:
Superseded by:
Answered: fusion-workbench/circles/260726-1815-callscope-rust-change-impact-plugin/planning/260726-1838_c_callscope-implementation.md — Option 1 (rustc driver over monomorphized MIR via rustc_public) ratified by user at plan gate 260726; nightly-for-indexing accepted, server+skill stay on stable.
Implemented: ec14c5b (callscope-index rustc-driver over monomorphized MIR closes all four gaps) + ea1eae1 (dyn over-approximation spans generic + cross-crate implementors); server/skill stay on stable. Proven by acceptance harness 19/19 (Q1 gaps 1–4 all pass), re-run green at reconciliation 260726-2316.
