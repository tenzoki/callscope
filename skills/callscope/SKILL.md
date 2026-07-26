---
name: callscope
description: >-
  Use before modifying a Rust function in a cargo workspace to get
  compiler-grounded change-impact answers — who calls it, what it reaches, and
  exactly which tests to run. Prefer this over grep or rust-analyzer for
  "what breaks if I change this", because callscope resolves what the compiled
  program executes (generics, dyn dispatch, closures, async, tests) rather than
  matching names in source text. Read it whenever you are about to change a
  function signature or behavior and need to size the blast radius before editing.
---

# callscope: compiler-grounded change impact for Rust

callscope answers change-impact questions from a pre-built index of what a Rust
workspace's compiler actually resolves — not text search, not name lookup. Use
it before you edit a function so you change the right callers and run exactly the
right tests instead of the whole suite or a guessed subset.

The index is on disk (`.callscope/index.bin` + `.callscope/manifest.json`). The
MCP server reads it and never touches the compiler. Every tool returns one
**Envelope** whose flags tell you how far to trust the answer. Reading those
flags is the point of this skill.

## Step 0 — index first, always

Before any change-impact question, make sure the workspace is indexed:

```
callscope-index <path-to-workspace>
```

This builds `.callscope/index.bin` and `.callscope/manifest.json`. The MCP tools
read that index; with no index there is nothing to answer from. Indexing is
repeatable within an editing session — re-run it whenever sources change (see
"Staleness" below). Indexing needs the pinned nightly toolchain; querying does
not.

## The eight tools, mapped to your real questions

Every symbol-taking tool accepts a name, a fully-qualified path, or a numeric id.

| Your question | Tool | Cap |
|---|---|---|
| "What symbol does this name mean?" | `resolve_symbol` | C1 |
| "Who calls this / what does it call, one hop?" | `direct_calls` | C2 |
| "Everything this reaches, or everything that reaches it?" | `reachability` | C3 |
| "How does A get to B?" | `call_paths` | C4 |
| "Which tests must I run after changing this?" | `affected_tests` | C5 |
| "Is there unsafe code downstream of this?" | `reachable_unsafe` | C6 |
| "What breaks if I change this?" (callers + tests, one call) | `impact` | C7 |
| "Show me the neighborhood as a graph" | `neighborhood_graph` | C8 |

- **`resolve_symbol` (C1)** turns a name or fragment into a concrete symbol with
  its characteristics (test, public, async, generic, foreign). It never guesses:
  an ambiguous fragment comes back as a **candidate set**, not one pick. Choose
  the right one by its fully-qualified path or id before calling anything else.
- **`direct_calls` (C2)** — the immediate callers and callees, one hop each way.
- **`reachability` (C3)** — the transitive set: `forward` = everything the symbol
  can reach, `backward` = everything that can reach it.
- **`call_paths` (C4)** — the concrete call chains from one symbol to another,
  bounded by depth and count.
- **`affected_tests` (C5)** — the tests that transitively reach the symbol. This
  is what you run after changing it. It includes tests that reach the symbol only
  through generic or `dyn` dispatch and never name it.
- **`reachable_unsafe` (C6)** — the unsafe-using symbols reachable forward, so you
  know whether a change lands near unsafe code.
- **`impact` (C7)** — the combined "what breaks if I change this": direct callers
  **plus** affected tests in one answer. Reach for this first on a change-impact
  question.
- **`neighborhood_graph` (C8)** — a Mermaid `flowchart` you read inline. Dashed
  edges are `dyn` dispatch; thick edges leave the workspace.

## The intended sequence

Guiding example: *"I want to change `parser::normalize_token` to return `Result`.
What's affected?"*

1. **Index** if you haven't this session: `callscope-index <workspace>`.
2. **Resolve** the target: `resolve_symbol { name: "normalize_token" }`. Confirm
   you have the right symbol (`parser::normalize_token`) by its fully-qualified
   path. If several match, disambiguate before continuing.
3. **Ask for impact**: `impact { symbol: "parser::normalize_token" }` — this
   returns the direct callers to update and the affected tests in one call.
   (Equivalent longhand: `affected_tests` + `direct_calls`.)
4. **Read the affected tests**, then **run exactly those** — not the whole suite.

For this example the affected-tests answer includes a test that reaches
`normalize_token` only through generic trait dispatch (`run_generic::<Simple>`)
and a test in the separate `parser` integration-test target. Neither spells the
function's name at the test; grep would miss both. That is the whole reason to
use callscope here.

## Reading the Envelope flags

An answer is only as trustworthy as its flags say. Check them before acting.

### Over-approximation — `over_approximated` (Q2)

When set, the answer is a **superset** of what provably executes — do not treat
it as exact. The canonical case is `DynDispatch`: the walk crossed a
`&dyn Trait` call, which is resolved at run time, so callscope widens it to
**every workspace implementor of that trait**.

Concretely, if a test reaches your target through `&dyn Tokenizer` and both
`Simple` and `Fancy` implement `Tokenizer`, the answer folds in the paths through
both, because either could run. `over_approximated` carries
`DynDispatch { trait_path: "parser::Tokenizer", implementor_count: 2 }`. Read
that as: "reachable via **any** `Tokenizer` implementor" — some listed paths may
not fire at run time, but none that could fire is missing. Prefer running a
too-large test set over an unsound one.

### Staleness — `stale` (Q6)

When set, the index predates the current source. `stale` carries
`diverged_files` — exactly which `.rs` files changed since indexing. The answer
still comes back (from the old index), but **do not trust it**: re-run
`callscope-index <workspace>` and ask again. The staleness check runs on every
tool call, so a missing `stale` field means the index matched the sources at
answer time.

### Truncation and totals — `truncated` + `total` (Q4)

`total` is always the true count. When `truncated` is `true`, the returned list
was capped below `total` for size. To see the rest, raise the tool's `limit`
(or `node_limit` for the graph, `max_paths` for call paths), or narrow the query
to a more specific symbol. Never read a truncated list as complete.

### Boundary — `boundary_applies`

When `true`, the walk reached the edge of your own workspace crates. v1 does
**not** follow calls through third-party dependency code — a callback handed to a
framework, for instance, stops at the boundary and its downstream is not
enumerated. The answer is complete up to that edge, not beyond it. Keep this in
mind when a change's real reach runs through a dependency.

## The honest framing

callscope reflects what the compiler resolves for the workspace: it closes the
gaps text search can't — generic instantiation, closure and async bodies,
macro-expanded `#[test]` / `#[tokio::test]` harnesses, and `unsafe` attribution.
That makes it far more precise than grep or a language server for "what
executes."

Two limits are honest and permanent for v1, and the flags above surface both:
answers **over-approximate at `dyn` sites** (`over_approximated`), and they
**stop at the workspace boundary** (`boundary_applies`). Trust the exact answers;
read the flagged ones as the superset or the bounded answer they are.
