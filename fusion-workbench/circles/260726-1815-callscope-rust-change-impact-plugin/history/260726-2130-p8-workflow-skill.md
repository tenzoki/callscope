# P8 — callscope workflow skill (skills/callscope/SKILL.md)

**Date:** 260726-2130
**Agent:** coder
**Status:** Complete
**Plan:** planning/260726-1838_p_callscope-implementation.md step 8
**Task:** P8 in tasklist.md

## What was built

`skills/callscope/SKILL.md` — a standalone Markdown skill (new `skills/callscope/`
directory) that teaches an AI coding agent the intended callscope workflow for
Rust change-impact work, including how to read uncertainty and staleness. No
build; the file depends on nothing in the crates.

Structure:
- **YAML frontmatter** — `name: callscope`, `description` framed around *when to
  use it* (before modifying a Rust function, to find callers/affected tests;
  prefer over grep/rust-analyzer for "what breaks if I change this").
- **Step 0 — index first** — `callscope-index <workspace>` builds
  `.callscope/index.bin` + `manifest.json`; the MCP server reads that index;
  nightly needed only for indexing.
- **The eight tools mapped to real questions** — a question→tool→cap table plus a
  one-line-each gloss of `resolve_symbol` (C1, candidate-set / no guessing),
  `direct_calls` (C2), `reachability` (C3), `call_paths` (C4), `affected_tests`
  (C5), `reachable_unsafe` (C6), `impact` (C7, reach-for-first), and
  `neighborhood_graph` (C8, inline Mermaid).
- **The intended sequence** — grounded in the guiding example (change
  `parser::normalize_token` to return `Result`): index → resolve → impact → read
  affected tests → run exactly those. Names the two load-bearing answers (the
  `run_generic::<Simple>` generic-dispatch test and the separate integration-test
  target) that grep would miss.
- **Reading the Envelope flags** — one subsection each for `over_approximated`
  (Q2, with the concrete `DynDispatch { trait_path, implementor_count }` /
  Simple+Fancy case, read as "any workspace implementor"), `stale` (Q6, re-index
  on `diverged_files`), `truncated`+`total` (Q4, raise limit / narrow), and
  `boundary_applies` (v1 stops at workspace edge, dependency callbacks not
  followed).
- **The honest framing** — compiler-resolved, not name matching; two permanent
  v1 limits (dyn over-approximation, workspace boundary) both surfaced by flags.

## Key decisions and findings

- **Grounded in the real implementation, not the plan's prose.** Read
  `envelope.rs` and `tools.rs` so the skill uses the actual field names
  (`over_approximated`, `stale.diverged_files`, `truncated`+`total`,
  `boundary_applies`) and the real `Reason::DynDispatch { trait_path,
  implementor_count }` shape, and the real tool names/limits
  (`limit`/`node_limit`/`max_paths`). The MCP server's own `INSTRUCTIONS`
  constant in `tools.rs` already frames the flags this way; the skill matches
  that wording so the two do not diverge.
- **Fixture names verified** against `fixtures/workspace/parser/src/lib.rs`
  (`normalize_token`, `Tokenizer`, `Simple`/`Fancy`, `run_generic`, `run_dyn`,
  the `#[tokio::test]`, and the `tests/integration.rs` dyn-dispatch test) so the
  guiding example is concrete and correct rather than paraphrased.
- **Scope respected.** Only `skills/callscope/SKILL.md` created. Did not touch any
  crate, Cargo.toml/lock, the fixture, or the fusion-workbench beyond the two
  tracking edits. This is a project deliverable skill, not a fusion skill.

## Build / test / verify

- No build (standalone Markdown).
- Frontmatter validated with `yaml.safe_load`: parses; keys `name`,
  `description` present; `name: callscope`.
- Coverage grep confirmed all 8 tool names, `over_approximated`/`stale`/
  `truncated`/`boundary_applies` (Q2/Q4/Q6 + boundary), `DynDispatch`, and
  `callscope-index` are all present.

## Follow-ups / notes for downstream tasks

- P9 (plugin packaging) can now proceed: `plugin.json` should declare this
  bundled skill at `skills/callscope/`, alongside the `callscope-mcp` MCP server
  launch and the one-time `callscope-index` command / nightly prerequisite.
