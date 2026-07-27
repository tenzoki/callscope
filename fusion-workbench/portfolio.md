# Portfolio

**Generated:** 260726-2322 (by playmaker session 260726-2322)
**Domain bias:** code

## Active (_t_)

(none) — no Circle is currently active. The pointer file `.active-circle` is correctly cleared.

## Anticipated (_a_) — ranked

(none) — there is no anticipated Circle to rank or recommend.

To start new work, draft a Directive with `/fusion:direct <one-line goal>`, which files a new anticipated Circle you can then activate.

## Recently closed (_c_ / _b_)

1. **`260726-1815-callscope-rust-change-impact-plugin`** — closed coherent (`_c_`), 260726-2316.
   Delivered the callscope Claude Code plugin (an MCP server plus a workflow skill) giving an AI coding agent compiler-grounded change-impact answers about a Rust cargo workspace. Definition of done met: capabilities C1 through C8 and quality requirements Q1 through Q6 are demonstrable against the fixture, including the guiding example (the affected-tests answer for `parser::normalize_token` includes both the generic-trait-dispatch test and the separate-crate integration test). Delivered over 3 Turns and 13 commits (`b602191..01fcf60`); acceptance harness 19 of 19 green, re-verified at reconciliation. One mid-run course-correction (Turn 2) closed two silent trait-object under-approximations before closure.

## Archived (_s_ / _d_)

(none) — no superseded or deferred Circles.

## Warnings

No mechanical warnings: the pointer is consistent (absent, with no active Circle — the normal post-closure state), no dependency cycles exist, and no parent Grounding was left stale (the closed Circle had no dependents).

Informational — open follow-ups recorded inside the closed callscope Circle, all marked non-blocking at closure. They do not require action, but if you want any of them done, capture a new Circle for it (they will not be picked up automatically):

- 4 open defect issues: FNV-1a hash duplicated between the schema and fingerprint crates; Mermaid label escaping unverified against the v11 grammar; MCP mirror structs duplicate the core query payloads; staleness check rehashes the whole workspace on every request.
- 2 open design decisions: whether the workspace-boundary flag should fire on calls into the standard library on forward walks; the MCP server's behaviour and user experience when no index is present at startup.

References for these follow-ups: `fusion-workbench/circles/260726-1815-callscope-rust-change-impact-plugin/issues/` and `.../decisions/` (the `_o_`-marked files).
