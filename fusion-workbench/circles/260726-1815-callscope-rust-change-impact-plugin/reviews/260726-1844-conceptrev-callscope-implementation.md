# Concept Evaluation: callscope implementation plan

**Date:** 2026-07-26 18:44
**Target:** `fusion-workbench/circles/260726-1815-callscope-rust-change-impact-plugin/planning/260726-1838_o_callscope-implementation.md`
**Verdict:** clean
**Diagrams evaluated:** 2  |  **Validation:** by-tool (mmdc 11.4.2) + by-reading for the two Mermaid v11.13.0 tripwires

## Verdict

The plan's design reads coherently in both of its diagrams. The component-shape graph is a small acyclic graph with visible layering (two subgraphs, explicit top-down direction), a maximum fan-out of 2, no cycles, and no orphans; the indexing-pipeline graph is a clean seven-stage linear chain. Neither graph hides a god-node, a silent cycle, or a missing layer. Both parsed under the local mermaid-cli, and both clear the two parser tripwires this project has hit before (`\n` inside labels, reserved-word node ids). Note one factual correction: the plan carries **two** Mermaid diagrams, not three. The C8 capability renders Mermaid at runtime, but no third diagram is embedded in this document.

## Per-diagram measurements

| # | Name | Type | Nodes | Edges | Max fan-out | Max fan-in | Cycles | Layered | Verdict |
|---|------|------|-------|-------|-------------|------------|--------|---------|---------|
| 1 | Component shape | flowchart TD | 7 | 8 | 2 (`core`, `manifest`) | 4 (`mcp`) | 0 | yes (2 subgraphs) | clean |
| 2 | Indexing pipeline | flowchart LR | 7 | 6 | 1 | 1 | 0 | linear | clean |

## Findings

No substantive findings. The items below are observations that do not change the verdict.

**Diagram 1 — `mcp` fan-in of 4, examined and cleared (not a god-node).** The MCP-server node `mcp` receives four incoming edges (`core`, `disk`, `manifest`, `skill`), the highest convergence in either graph. This is a legitimate serving hub, not a god-object: each edge is a distinct, labeled relationship — a code dependency (`core` provides query algorithms and the envelope), a data read (`disk` holds the index), a launch (`manifest` starts it), and agent guidance (`skill` points to it). The node's own fan-out is zero; it is a terminal sink, exactly what an MCP server should be. Because the edges are labeled, a reader sees four different relations converging, not one overloaded owner.

**Diagram 1 — mixed relationship semantics on one graph (acceptable for an architecture-shape diagram).** The edges blend build-time (`compiled by`, `declares index command`), runtime (`launches`, `read by`, `guides agent to`), and code-dependency (`schema + fingerprint`, `query algos + envelope`) relations. Per the one-diagram-one-concern rule this is worth noting, but it does not rise to overloaded: the graph's single concern is "how the components relate," every edge is labeled so the concern of each is legible, and the arrowhead consistently lands on the acting or dependent component. Splitting it would lose the at-a-glance shape without adding clarity. Leave as is.

**Diagram 2 — unlabeled edges, correctly so.** The six edges carry no labels. On a semantic graph that would hide the design, but this is a linear transformation pipeline where every edge means the same thing ("then this stage"). A label on each would be redundant. No action.

**Mermaid v11.13.0 tripwire check — both clear.**
- **`\n` inside labels:** none. Every multi-line label in both diagrams uses `<br/>` (for example `"workflow skill<br/>(index-first, ...)"`). Correct for the stricter parser.
- **Reserved-word node ids:** none. Node ids in diagram 1 are `skill`, `manifest`, `core`, `index`, `mcp`, `disk`, `target`; subgraph ids are `plugin`, `ws`; diagram 2 uses `A`–`G`. None collides with a Mermaid keyword. In particular `index` is a plain identifier, not a reserved word, so it is safe (unlike the earlier `graph` collision this project hit).
- **Validator caveat:** the local mermaid-cli is 11.4.2, one minor line below the 11.13.0 render target. Both diagrams parsed there, and the two 11.13.0-specific tripwires were additionally checked by reading. I did not exercise the exact 11.13.0 parser.

## What a clean redraw would require

Not applicable — verdict is clean. No structural change is needed. If the planner wants to tighten diagram 1 cosmetically, grouping the three build-time edges visually from the runtime edges would sharpen the read, but that is optional polish, not a design fix.
