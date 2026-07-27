# Coder session — P10 fixture workspace

**Date:** 2026-07-26 19:04
**Agent:** coder
**Task:** P10 (plan `260726-1838_p_callscope-implementation.md`, step 10)
**Status:** Complete

## Scope

The acceptance substrate mandated by `problem.md` §6. A small, self-contained
cargo workspace under `fixtures/workspace/` that exercises every Rust construct
the acceptance sketch enumerates and makes the guiding example answerable. No
callscope code touched — fixture files only, plus one surgical edit to the root
`Cargo.toml` and one `.gitignore` line.

## What was created

- `fixtures/workspace/Cargo.toml` — its own `[workspace]` (`resolver = "2"`,
  member `parser`). Independent of the main callscope workspace one level up.
- `fixtures/workspace/parser/Cargo.toml` — `parser` lib crate. Dev-dependency
  `tokio` (features `rt`, `macros`) solely for the `#[tokio::test]` harness.
- `fixtures/workspace/parser/src/lib.rs` — the library. Contents mapped to the
  four source-vs-execution gaps:
  - `pub fn normalize_token(&str) -> String` — THE guiding-example target.
  - `pub trait Tokenizer { fn tokenize(&self, &str) -> Vec<String>; }`.
  - Two implementors `Simple` and `Fancy`, both calling `normalize_token`.
    `Simple::tokenize` reaches it through a closure body
    (`.map(|t| normalize_token(t))`) — gap 3, closure attribution.
  - `pub fn run_generic<T: Tokenizer>(&T, &str)` — monomorphization site, gap 1.
  - `pub fn run_dyn(&dyn Tokenizer, &str)` — `dyn` dispatch site, gap 2.
  - `pub async fn tokenize_async(&str)` — async body reaching the target via
    `run_generic(&Simple, ..)`, gap 3.
  - Private `ensure_round_trip` with a genuinely `unsafe {}` raw-pointer
    round-trip, reachable from public `normalize_token` — C6. Trivially safe in
    practice (reconstructs the string's own live byte slice, re-validates).
  - Unit tests: `#[test] normalizes_directly` (names the target),
    `#[test] reaches_via_generic_dispatch` (only via `run_generic`, never names
    the target), `#[test] reaches_via_dyn_dispatch`, and
    `#[tokio::test] async_reaches_target` — the macro-expanded harness, gap 4.
- `fixtures/workspace/parser/tests/integration.rs` — SEPARATE integration-test
  target (own compiled crate). Both tests reach `normalize_token` ONLY through
  generic (`run_generic`) or `dyn` (`run_dyn`) dispatch; neither names the
  target. This is the load-bearing §6 acceptance answer (different-crate test).

## Root-workspace edits (surgical)

- `Cargo.toml` (project root): added `exclude = ["fixtures/workspace"]` to the
  `[workspace]` table (no prior `exclude` key existed), with a two-line comment.
  Isolates the fixture as its own workspace so the main build ignores it.
- `.gitignore`: added `/fixtures/workspace/target` (existing `/target` is
  root-anchored and does not cover the nested fixture build dir).

## Verification (both commands mandated by the task)

- `cd fixtures/workspace && cargo test` → PASS. tokio fetched from crates.io;
  fixture `Cargo.lock` written. Results:
  - lib unittests: 4 passed (`normalizes_directly`, `reaches_via_generic_dispatch`,
    `reaches_via_dyn_dispatch`, `async_reaches_target`).
  - integration.rs: 2 passed (`integration_reaches_via_generic`,
    `integration_reaches_via_dyn`).
  - doc-tests: 0. Total: 6 passed, 0 failed.
- `cargo build` at PROJECT ROOT → `Finished dev profile ... in 0.11s`. The three
  callscope crates build; the fixture is not pulled in — the `exclude` edit
  isolates it correctly.

Both ran under the workspace nightly pin (`nightly-2026-07-26`), which the
fixture inherits from the root `rust-toolchain.toml`; the fixture uses no
nightly-only features and would build on stable too.
