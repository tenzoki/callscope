# Reconciliation — 260726-2316 (final, Phase 3)

**Agent:** reconciler
**Domain:** code
**Circle:** 260726-1815-callscope-rust-change-impact-plugin
**Scope:** final reconciliation at session convergence — verify all tracking against ground truth, compute the three-edge Coherence verdict.

## Counts

- Plans reviewed: 1; updated: 1 (Status → Complete, marker `_p_` → `_c_`, Reconciliation Log added).
- Issues reviewed: 9; updated: 4 (reconciliation evidence appended to the open set; markers unchanged).
- Decisions reviewed: 6; updated: 3 (advanced `_a_` → `_i_`).
- Circle record: 1 (Turn log filled with the three Turns; stale header fields corrected; marker left `_t_` for the orchestrator's Phase-4 transition).
- New issues filed: 0.

## Ground-truth verification

**Empirical acceptance run** (not trusted from history): rebuilt `callscope-index` on the pinned `nightly-2026-07-26` and ran `cargo test -p callscope-mcp --test acceptance --nocapture`. Result: **19/19 checks passed**, test `ok`, 7.26s. Fixture index 21 symbols / 93 edges, byte-deterministic across two runs. The load-bearing §6 answer holds: `affected_tests(parser::normalize_token)` returns both the in-crate generic-dispatch test and the separate-crate integration test.

**Files verified on disk:** three-crate workspace, `rust-toolchain.toml` (nightly + rustc-dev), workflow skill, `.claude-plugin/plugin.json`, `.mcp.json`, README, fixture (parser + ext_tokenizer + integration target). Git: 13 commits `b602191..01fcf60`, working tree clean.

**No drift.** Built code matches the plan's Approach and Data Structures: one `Envelope<T>` (stale / over_approximated / truncated+total / boundary_applies), three crates split by toolchain need, serde_json on-disk format.

## Key findings

- All 11 plan steps genuinely `[DONE]`; plan closed `_c_`.
- All 5 `_c_` issues genuinely resolved, corroborated by reviews + histories + the empirical run:
  - `call-paths-false-positive-truncated` + `call-paths-can-drop-over-approx-flag-when-truncated` (CR1, `dbf0f50`) — confirmed by Turn-2 review and acceptance C4/Q2/Q4.
  - `symbolid-collision-has-no-detection-silent-merge` (`ec14c5b`) — fail-on-collision guard + 2 unit tests confirmed.
  - `emit-virtual-empty-genericargs-drops-generic-implementors` + `emit-virtual-only-enumerates-local-crate-impls` (FIX-DYN `ea1eae1`) — acceptance DYN check confirms four-implementor coverage (generic `Wrapper<T>` + cross-crate `ext_tokenizer::Shouty`), implementor_count=4.
- All 4 `_o_` issues genuinely still open (ground-truth confirmed): FNV still duplicated in schema.rs:51; fingerprint still hash-all; core query payloads still un-serializable; mermaid v11 render-check still absent. All Low severity, none blocks closure.
- Decisions advanced `_a_` → `_i_` (answers now realised in committed code, proven green): `260726-1838` analysis-technique/toolchain (`ec14c5b`+`ea1eae1`), `260726-1914` staleness-hash-all (`e274a71`), `260726-2253` generic-implementor-representation (`ea1eae1`). Decision surface is now 4 `_i_` / 2 `_o_`.

## Open decision surface (post-v1 follow-ups, genuinely open)

- `260726-2210_o_boundary-applies-triggered-by-std-calls-on-forward-walks.md` — should the boundary flag fire on ordinary std calls or only on chain-continuing dependency crossings? P11 followed the review's guidance (asserts the flag is *set* where a third-party edge is genuinely crossed, never asserts `== false` on a forward walk). Signal-quality refinement, correctly a decision.
- `260726-2210_o_mcp-server-behavior-when-no-index-present.md` — hard-fail vs. serve a no-index error envelope on first run. First-run UX; correctly a decision.

Both are correctly filed as decisions (choice points with options), not misfiled defects. Neither contradicts the Directive's definition of done.

## Misfiled — should be a decision

None. The 4 open issues are genuine defects/verification gaps (all "go fix it"); the 2 open decisions are correctly in the decision store.

## Coherence verdict

Aggregate: **coherent**. Written to the orchestrator session history `## Coherence` section (`history/260726-1834-orchestrator-session.md`). Rebalance recommendation: none.

- Artifact↔Grounding: 11/11 plan steps verified on disk, acceptance 19/19 re-run green, 0 drift; 4 open coderev issues (all Low, non-blocking) + 2 open design decisions (post-v1).
- Artifact↔Directive: definition of done met — C1–C8 + Q1–Q6 demonstrable against the fixture incl. the guiding example; all 13 commits `b602191..01fcf60` move toward the Directive, none orthogonal.
- Grounding↔Directive: 4 implemented decisions consistent with the Directive / 0 conflicting; the 2 open decisions and the generic-implementor residual are documented honest limitations, not contradictions.
