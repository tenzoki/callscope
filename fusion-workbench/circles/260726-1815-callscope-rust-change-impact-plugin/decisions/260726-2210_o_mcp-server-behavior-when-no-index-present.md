# How should callscope-mcp behave when launched with no index on disk?

---
**Domain:** code
**Status:** open
**Filed by:** coderev
**Cross-references:** crates/callscope-mcp/src/main.rs:46-61 (startup + load), crates/callscope-mcp/src/state.rs:116-148 (IndexState::load), .mcp.json (auto-launch), skills/callscope/SKILL.md (index-first workflow)

---

## Question

`callscope-mcp` loads the index once at start-up and propagates any load error
out of `main` (`main.rs:54` — `let state = IndexState::load(&workspace)?;`). If
`<workspace>/.callscope/index.bin` does not exist (the user has never run
`callscope-index`), `load` returns `Err`, `main` returns `Err`, and the process
exits before `serve` is ever called.

Delivery form is a Claude Code plugin whose MCP server is auto-launched by
`.mcp.json`. When the server exits during start-up, the MCP handshake fails and
**none of the eight tools register**. The agent then has no callscope tools and
no in-band signal explaining why — the "run `callscope-index` first" guidance
lives in the skill, but the skill cannot compensate for tools that never
appeared. First-run experience on an unindexed workspace is a silent failure.

## Options

1. **Keep hard-fail at start-up (current).**
   - Pros: simplest; the server never serves a half-truth from a missing index.
   - Cons: on the common first-run case the plugin appears broken with no
     actionable message reaching the agent.
2. **Start the server even with no index; have every tool return a well-formed
   error envelope ("no index found at <path>; run `callscope-index <workspace>`").**
   - Pros: the agent gets an actionable, in-band instruction through the normal
     tool-result channel; the plugin looks alive and self-explains.
   - Cons: tools must all handle the no-index state; slightly more code.
3. **Start the server, and on first tool call attempt to build the index
   automatically (or instruct exactly how).**
   - Pros: smoothest UX.
   - Cons: auto-indexing needs the nightly toolchain and can be slow; doing heavy
     work implicitly inside a tool call is surprising. Likely v2.

## Constraints

- Must not serve stale or fabricated answers when no index exists — a no-index
  state must be visibly a no-index state, never an empty "nothing found" answer
  that reads as a real result.
- Stable-toolchain only for the server; option 3's auto-index would shell out to
  the nightly indexer, not link it.

## Recommendation

Option 2 for v1: start unconditionally and return an explicit no-index error
envelope from each tool, carrying the exact `callscope-index <workspace>` command.
It turns a silent plugin failure into a one-line instruction the agent can act on,
with no new toolchain dependency. Option 3 is the v2 convenience.

---
Answered:
Implemented:
Deferred:
Superseded by:
