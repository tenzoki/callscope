# Coder session — P1 workspace scaffold + nightly toolchain pin

**Date:** 2026-07-26 18:57
**Agent:** coder
**Task:** P1 (plan `260726-1838_p_callscope-implementation.md`, step 1)
**Status:** Complete

## Scope

Scaffold-only. Three-crate cargo workspace plus the nightly toolchain pin. No
module logic, no fixture, no MCP/index implementation (later tasks). All module
files in callscope-core are comment-only stubs.

## What was created

- `Cargo.toml` — workspace root, `resolver = "2"`, members are the three crates,
  `[workspace.package]` (version 0.1.0, edition 2021), `[workspace.dependencies]`
  (internal `callscope-core` by path, `serde` with derive, `serde_json`).
- `crates/callscope-core/` — stable lib. Deps: serde (derive), serde_json.
  `src/lib.rs` declares `pub mod schema; envelope; fingerprint; query; mermaid;`.
  Each module file is a single `// P<n>: ...` comment stub.
- `crates/callscope-index/` — binary crate depending on callscope-core;
  `main.rs` prints "not yet implemented" and exits 0.
- `crates/callscope-mcp/` — binary crate depending on callscope-core; same stub
  main. `rmcp` deliberately NOT added yet (P7 wires it).
- `rust-toolchain.toml` — pins `nightly-2026-07-26`, components `rustc-dev`,
  `llvm-tools-preview`, `rust-src`, profile minimal. Comment records the pin
  exists for callscope-index's rustc_public linkage and that only indexing needs
  nightly.
- `.gitignore` — `/target`, `**/*.rs.bk`, `.callscope/`, OS cruft.

## Toolchain note

Only stable was installed at start. Installed both `nightly` (latest) and the
dated `nightly-2026-07-26` (rustc 1.99.0-nightly, 008fa22ce 2026-07-25) with the
three components, so the dated pin references an actually-installed toolchain.
The latest nightly on the server today is dated 2026-07-26, which is the pin.

## Verification

- `rustup show active-toolchain` → `nightly-2026-07-26-aarch64-apple-darwin`
  (overridden by rust-toolchain.toml). Pin resolves to an installed toolchain.
- `cargo build` at workspace root → `Finished dev profile ... in 8.18s`. All
  three crates compiled green under the pin. serde/serde_json fetched from
  crates.io; Cargo.lock written.
- Both stub binaries run and exit 0.
