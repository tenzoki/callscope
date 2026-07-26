# callscope

Compiler-grounded change-impact answers for Rust cargo workspaces, as a Claude
Code plugin.

Before an AI coding agent edits a Rust function, it needs precise answers: who
calls this, what does it reach, and exactly which tests must run to validate the
change. callscope answers those from what the **compiler actually resolves** for
the workspace, not from names matched in source text.

## Why not grep or a language server

Source-level tools see names, not execution. In Rust the gap is unusually wide,
and callscope closes the four cases text search and rust-analyzer miss:

- **Generics and monomorphization.** A call inside `run_generic<T: Tokenizer>`
  resolves to whichever concrete `tokenize` the caller instantiated. The source
  has one call; the compiled program has one per instantiation.
- **Trait objects (`dyn Trait`).** A call through `&dyn Tokenizer` dispatches at
  run time. The honest static answer is "any workspace implementor", and
  callscope reports it as exactly that, flagged.
- **Closures and `async fn`.** The compiler lowers these to separate anonymous
  functions. A call inside `.map(|t| normalize_token(t))` belongs to the
  enclosing function; callscope folds it back.
- **Macro-expanded tests and `unsafe`.** `#[test]` and `#[tokio::test]` expand
  into generated harness items; `unsafe {}` is a source marker. callscope reads
  both from the compiler, not from pattern matching.

## What ships

Three cargo crates plus the plugin packaging:

| Component | Role | Toolchain |
|---|---|---|
| `callscope-core` | Index schema, staleness fingerprint, query algorithms, the one output envelope. Links no compiler internals. | stable |
| `callscope-index` | The indexer. The **only** crate that links the compiler (`rustc_public` / `rustc-dev`) to read monomorphized reachability. | nightly |
| `callscope-mcp` | The stdio MCP server exposing C1–C8 as tools. Reads the index; never links the compiler. | stable |
| `skills/callscope/SKILL.md` | Workflow skill: index first, read the uncertainty and staleness flags, run exactly the affected tests. | — |

## Install and build

The workspace builds with cargo:

```
cargo build --release
```

That produces the query-time server at `target/release/callscope-mcp` — the
binary `.mcp.json` launches — and the indexer at `target/release/callscope-index`.

**Indexing requires the pinned nightly toolchain.** Only `callscope-index` needs
it: it links compiler internals shipped in the `rustc-dev` component. The server
and the skill run on stable. The exact channel and components are pinned in
[`rust-toolchain.toml`](./rust-toolchain.toml) (a `nightly-*` channel plus
`rustc-dev`, `llvm-tools-preview`, `rust-src`); rustup installs them
automatically when you build in this directory. The generated index manifest
records the toolchain actually used, so a later toolchain drift is detectable.

## Usage

### 1. Index the target workspace (one time, then after edits)

```
callscope-index <path-to-your-cargo-workspace>
```

This builds `<workspace>/.callscope/index.bin` and
`<workspace>/.callscope/manifest.json`. Indexing runs in test mode so `#[test]`
items seed the reachability roots. It is repeatable within an editing session;
re-run it whenever an answer reports staleness (see below).

### 2. Query through the MCP server

When the plugin is enabled, Claude Code launches `callscope-mcp` automatically
(per `.mcp.json`) and its eight tools appear alongside your other MCP tools. The
server resolves which workspace to serve, in order:

1. the first positional CLI argument, if given;
2. otherwise the `CALLSCOPE_WORKSPACE` environment variable;
3. otherwise the current working directory.

`.mcp.json` launches the built release binary and passes `CALLSCOPE_WORKSPACE`
through from your environment. Leave it unset to serve the current working
directory (Claude Code's project root, the common case); set it to point the
server at a workspace elsewhere.

**Dev alternative** (no separate build step): replace the `command` in
`.mcp.json`, or run manually, with

```
cargo run --release -p callscope-mcp -- <path-to-your-cargo-workspace>
```

The skill [`skills/callscope/SKILL.md`](./skills/callscope/SKILL.md) teaches the
agent the intended sequence — resolve the target, ask for `impact`, read the
affected tests, and honor the staleness and over-approximation flags.

## Capabilities (C1–C8) and their tools

Every tool returns the same **Envelope**, carrying the uncertainty flags below.

| # | Capability | Tool |
|---|---|---|
| C1 | Resolve a name or fragment to a symbol, with its characteristics (test, public, async, generic, foreign). Never guesses — an ambiguous name comes back as a candidate set. | `resolve_symbol` |
| C2 | A function's direct callers and direct callees. | `direct_calls` |
| C3 | Transitive reachability, forward (what it reaches) and backward (what reaches it). | `reachability` |
| C4 | Enumerated call paths between two functions. | `call_paths` |
| C5 | Tests that transitively reach a function — what to run after changing it. | `affected_tests` |
| C6 | Unsafe code reachable from a function. | `reachable_unsafe` |
| C7 | Combined per-function impact: callers plus affected tests in one call. | `impact` |
| C8 | Renderable neighborhood as Mermaid `flowchart` text. | `neighborhood_graph` |

## Honest limitations

These are permanent for v1, and every answer surfaces them through Envelope flags:

- **`dyn`-dispatch answers over-approximate.** A walk that crosses a `&dyn Trait`
  call widens to every workspace implementor of that trait, because either could
  run. The result sets `over_approximated`; read it as "any workspace
  implementor", not as an exact list. Some listed paths may not fire at run time,
  but none that could fire is missing.
- **v1 stops at the workspace boundary.** Calls that pass through third-party
  dependency code — a callback handed to a framework, for example — are not
  followed past the edge of your own workspace crates. Such answers set
  `boundary_applies`: complete up to that edge, not beyond it.
- **Staleness is detectable; re-index after edits.** The server runs a source
  fingerprint on every request. When the index predates the sources, the answer
  still returns but sets `stale` with the exact diverged files. Re-run
  `callscope-index` and ask again before trusting it.
- **Only reachable items appear.** This is a "what executes" tool, not a name
  index: items unreachable from any root (including `#[test]` roots) are absent
  by design.

## Acceptance / demo target

The fixture at [`fixtures/workspace/`](./fixtures/workspace/) is the acceptance
substrate. It exercises generics, `dyn` dispatch, closures, `async fn`, unit and
`#[tokio::test]` tests, a separate integration-test crate, and reachable unsafe
code. The guiding example — changing `parser::normalize_token` to return
`Result` — is answerable against it: the affected-tests answer includes both a
test that reaches the function only through generic trait dispatch and a test in
the separate integration-test target. Neither names the function; grep would
miss both.
