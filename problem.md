# callscope: Precise Change-Impact Answers for Rust Workspaces

**Type:** Tender / requirements document
**Deliverable sought:** A Claude Code plugin named `callscope`
**Status:** Open for implementation proposals

## 1. Context

AI coding agents such as Claude Code routinely modify Rust cargo workspaces. Before touching a function, a competent agent needs precise answers to impact questions: who calls this function, what does it reach, and which tests must run to validate the change. Today the agent's view of the code is source text. It searches with grep-style tools and consults language-server tooling, and both operate on names as written in the source, not on what the compiled program actually executes.

## 2. The problem

Source-level tools cannot see what executes. Text search finds a name wherever it is spelled, including comments, strings, and unrelated functions that happen to share the name, and it misses every call that does not spell the name at the call site. Language-server tooling resolves references, but it answers "where is this name used", not "what can actually run this code". In Rust the gap between source and execution is unusually wide, for four language-specific reasons:

1. **Generics and monomorphization.** A generic function `run_generic<T: Tokenizer>` contains the call `tokenizer.tokenize(...)`. Which concrete `tokenize` runs depends on the type each caller instantiates the function with. The source contains one call; the compiled program contains one per instantiation. A source-level tool cannot report that a test calling `run_generic(&Simple, ...)` thereby reaches `Simple`'s `tokenize` implementation and everything behind it.
2. **Trait objects (`dyn Trait`).** A call through `&dyn Tokenizer` is dispatched at run time. The honest static answer is "any implementor of this trait", and computing the set of implementors across a whole workspace is itself beyond text search.
3. **Closures and `async fn`.** The compiler turns closure bodies and async functions into separate anonymous functions, detached from the function the user wrote. A call inside `input.split_whitespace().map(|t| normalize_token(t))` textually sits in a closure, yet the answer a reader needs is that the enclosing function calls `normalize_token`. Recovering that attribution requires undoing the compiler's lowering.
4. **Macro-expanded test harnesses and erased unsafety.** A `#[test]` attribute (including wrappers such as `#[tokio::test]`) expands into generated harness items whose names collide with the user's own functions, so "is this a test?" cannot be answered reliably by pattern matching. Similarly, an `unsafe {}` block is a source-level marker with no direct counterpart in the compiled program, so "does this body use unsafe code?" is equally out of reach for name-based tooling.

The consequence for agentic coding is concrete. Consider the guiding example for this tender: *"I want to change `parser::normalize_token` to return `Result`. What code and tests are affected?"* A correct answer includes a test that reaches the function only through generic trait dispatch and never mentions it by name. An agent that cannot compute this answer will run the whole test suite (slow), run a guessed subset (unsound), or reason from search output (both).

## 3. Required capabilities

The tool must answer the following questions about any indexed cargo workspace. These state *what* must be answerable, not *how*; the analysis approach is the implementer's choice (see section 6).

| # | Capability |
|---|---|
| C1 | Resolve a function name or name fragment to a unique symbol, with its characteristics: test, public, async, generic, foreign. |
| C2 | List a function's direct callers and direct callees. |
| C3 | Compute transitive reachability from a function, forward (what it can reach) and backward (what can reach it). |
| C4 | Enumerate call paths between two given functions. |
| C5 | List the tests that can, transitively, reach a given function. |
| C6 | List the unsafe code reachable from a given function. |
| C7 | Produce a combined per-function impact answer covering callers and affected tests in one call. |
| C8 | Produce a renderable call-graph visualization of the neighborhood around a function. |

## 4. Delivery form

The deliverable must be a **Claude Code plugin** named `callscope`: an MCP server exposing the capabilities of section 3 as tools, plus a skill that teaches the agent the intended workflow, including how to interpret uncertainty and staleness in results (see section 5). The choice of internal technique, implementation language aside from the plugin packaging, and analysis machinery is entirely up to the implementer.

## 5. Quality and behavioral requirements

These are requirements on the answers the tool gives, not on its internals.

- **Q1: Ground truth.** Answers must reflect what the Rust compiler actually resolves for the workspace, not name matching over source text. The four gaps in section 2 must all be closed: calls through generic instantiation, trait-object dispatch, closure and async bodies, and macro-expanded test items must be attributed correctly.
- **Q2: Visible uncertainty.** Wherever a result necessarily over-approximates (run-time dispatch through trait objects is the canonical case) or is truncated for size, the result must say so explicitly. The consuming LLM must be able to see that an answer is an over-approximation or incomplete; a silent best guess is a defect.
- **Q3: No silent name guessing.** When a name lookup is ambiguous, the tool must return the candidate symbols rather than picking one.
- **Q4: LLM-suitable output.** Results must be compact, structured output fit for consumption by an LLM: bounded in size, with totals reported when lists are capped.
- **Q5: Repeatable indexing.** Indexing a workspace must be repeatable after code changes without unreasonable cost, so an agent can re-index within a normal editing session.
- **Q6: Detectable staleness.** It must be possible to detect that answers come from an index that predates the current source state.

## 6. Acceptance sketch

The implementer must supply a small fixture workspace that exercises generics, `dyn` dispatch, closures, `async` functions, unit tests, integration tests in a separate crate, and unsafe code. Against this fixture, the guiding example of section 2 must be answered correctly: the affected-tests answer for the target function must include the test that reaches it only through generic trait dispatch, and a test from a different crate's integration-test target. The capability set C1 through C8 and the quality requirements Q1 through Q6 must each be demonstrable against this fixture.

## 7. Open choices and scope boundary

The following are explicitly the implementer's choice: the analysis technique, toolchain requirements, internal architecture, data formats, and indexing strategy.

One boundary may be drawn for a first version: tracing call chains *through* third-party dependency code (for example, a callback handed to a framework) may be declared out of scope. If the implementer draws this boundary, it must be documented honestly, and every answer whose completeness the boundary affects must state that the boundary applies.
