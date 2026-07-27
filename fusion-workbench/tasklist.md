# Task Queue — callscope

**Source plan:** circles/260726-1815-callscope-rust-change-impact-plugin/planning/260726-1838_p_callscope-implementation.md
**Generated:** 260726-1834 (orchestrator, mode=plan)
**All tasks route to:** coder

| # | Task | Deps | Status |
|---|------|------|--------|
| P1 | Workspace scaffold + nightly toolchain pin (3 crates, rust-toolchain.toml) | — | [x] |
| P2 | Index schema + `Envelope<T>` in callscope-core | P1 | [x] |
| P3 | Fingerprint + staleness module in callscope-core (Q5/Q6) | P2 | [x] |
| P5 | Graph query engine in callscope-core (C1–C7 algos) | P2 | [x] |
| P6 | C8 Mermaid neighborhood renderer in callscope-core | P5 | [x] |
| P10 | Fixture workspace (generics, dyn, closure, async, unsafe, tests) | — | [x] |
| P4 | callscope-index rustc-driver engine (nightly, gaps 1–4) | P2,P3 | [x] |
| P7 | callscope-mcp MCP server, 8 tools C1–C8 | P3,P5,P6 | [x] |
| P8 | Workflow skill (skills/callscope/SKILL.md) | P7 | [x] |
| P9 | Plugin packaging (plugin.json, .mcp.json, README) | P7,P8 | [x] |
| P11 | Acceptance harness C1–C8 + Q1–Q6 against fixture | P4,P7,P10 | [x] |

Dependency tiers (execution order): (P1, P10) → P2 → (P3, P5) → (P4, P6) → P7 → P8 → (P9, P11)
