# P7 — callscope-mcp MCP server (C1–C8)

**Date:** 260726-2108
**Agent:** coder
**Status:** Complete
**Plan:** planning/260726-1838_p_callscope-implementation.md step 7
**Task:** P7 in tasklist.md

## What was built

A stable-toolchain stdio MCP server (`crates/callscope-mcp`) that loads the on-disk
index once at start-up, runs the staleness fingerprint on every request, and exposes
the eight tools mapping to C1–C8. Every tool returns the shared `callscope-core`
`Envelope<T>` serialized to compact JSON as tool text content.

Files (all under `crates/callscope-mcp/`):
- `Cargo.toml` — added `rmcp = "2.2"` (features `server`, `transport-io`, `macros`),
  `tokio` (`macros`, `rt-multi-thread`), and `serde`/`serde_json`. P1 deliberately
  left rmcp out; this task wired it.
- `src/state.rs` — `IndexState`: loads `index.bin` + `manifest.json`, the staleness
  check (`compute_stale` → `fingerprint::diverged_files`), symbol resolution with the
  ambiguity outcome (`Resolved`), and the eight per-capability handlers. Reuses
  `callscope-core` `query.rs` and `mermaid::render_neighborhood` for all logic.
- `src/tools.rs` — the MCP wire layer: `CallscopeServer` with `#[tool_router]` /
  `#[tool_handler]`, the eight `#[tool]` methods, their JSON-schema'd input structs,
  and `get_info`.
- `src/main.rs` — `#[tokio::main]`; resolves the workspace root, loads the index,
  serves over stdio.
- `tests/handlers.rs` — integration test against a hand-built index (no real
  `.callscope/index.bin`, no MCP client).

Reused core unchanged; did not touch callscope-core, callscope-index, query.rs, the
fixture, or root Cargo.toml.

## Key decisions and findings

- **rmcp version/API.** Current stable is `rmcp` 2.2.0 (3.0 is still beta). Used the
  macro pattern: `#[tool_router]` on the tools impl (generates `Self::tool_router()`,
  read from a `tool_router: ToolRouter<Self>` field) and a separate
  `#[tool_handler] impl ServerHandler`. Tools are `async fn`s taking
  `Parameters<ArgStruct>` and returning `Result<CallToolResult, ErrorData>`. Two
  2.x-specific gotchas: the content type is `rmcp::model::ContentBlock` (not
  `Content`), and both `ServerInfo` (= `InitializeResult`) and `Implementation` are
  `#[non_exhaustive]`, so `get_info` mutates fields on a `default()` rather than using
  a struct literal.
- **Server identity** set to this crate (`callscope-mcp` 0.1.0); the SDK default would
  otherwise report the server as "rmcp".
- **Index location.** Resolved from argv[1], else `$CALLSCOPE_WORKSPACE`, else `.`.
  Index read from `<workspace>/.callscope/index.bin`, manifest from
  `<workspace>/.callscope/manifest.json`. The workspace root is needed anyway for the
  staleness fingerprint, so it (not a bare index path) is the launch input.
- **On-disk format.** `.bin` had no writer (P4) to constrain the reader, so I picked a
  format and recorded it: both files are JSON via `serde_json`. Filed as a decision
  record (`decisions/260726-2108_a_on-disk-index-serialization-format.md`) so P4's
  indexer adopts the same choice deliberately. No new dependency; matches how core's
  own schema tests already round-trip an `Index`.
- **Serializable payloads.** `callscope-core`'s query payloads `DirectCalls`,
  `CallPath`, `Impact` derive no `Serialize` (only `Symbol`/`Envelope`/`Reason`/
  `StaleInfo` do). Since core is out of scope, the server defines serializable mirrors
  (`DirectCallsOut`, `CallPathOut`, `ImpactOut`) and remaps the envelope onto them,
  preserving every uncertainty flag.
- **Ambiguity (Q3).** Symbol-taking tools resolve a name/fragment via C1. Precedence:
  numeric `SymbolId` → exact fq_path → fragment search. One match answers; zero or
  many returns the candidate set (same shape as `resolve_symbol`) instead of guessing.
  Disambiguate by passing the exact fully-qualified path or the numeric id.
- **Staleness (Q6).** Ran on every tool call. When sources diverged, `stale` is
  attached with the diverged files; the server still answers from the (stale) index
  rather than refusing.
- `Reason` is `#[non_exhaustive]`: the server never matches on it (it serializes it
  through), so no wildcard arm was needed.

## Build / test commands and results

- `cargo build -p callscope-mcp` → Finished, no warnings. (Runs under the pinned
  nightly `nightly-2026-07-26`, which builds this stable crate; the crate uses no
  nightly-only features and links no compiler internals. `cargo +stable` is blocked
  only because the installed stable is 1.80, below rmcp's MSRV — not because of
  anything in this crate.)
- `cargo test -p callscope-mcp` → 5 passed, 0 failed. Covers C1 resolution, C5 through
  a dyn-dispatch edge with the Q2 flag, C2/C7 serialization, C3/C6/C8, and the Q6
  staleness flip on a source edit.
- `cargo test -p callscope-core` → 46 passed (unchanged; confirms core untouched).
- End-to-end stdio smoke test (hand-crafted minimal index): `initialize` → OK
  (`serverInfo.name = callscope-mcp`); `tools/list` → all 8 tools; `tools/call`
  `affected_tests {symbol: "normalize_token"}` → returned the reaching test with
  `over_approximated: DynDispatch` (Q2) and `stale` (Q6) both present, `total:1`.

## Follow-ups / notes for downstream tasks

- P4 (`callscope-index`) must write `index.bin` + `manifest.json` as `serde_json` with
  the `callscope-core::schema` types — see the decision record cited above.
- P8 (skill) and P9 (packaging) can now proceed: launch is
  `callscope-mcp <workspace>` (or `$CALLSCOPE_WORKSPACE`) over stdio.
- clippy was not run: `cargo-clippy` is not installed for the pinned nightly. `cargo
  build` is warning-clean.
