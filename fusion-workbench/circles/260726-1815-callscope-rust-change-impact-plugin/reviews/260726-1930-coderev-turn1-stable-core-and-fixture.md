# Code Review — Turn 1: callscope stable core + acceptance fixture

**Sender:** coderev
**Date:** 260726-1930
**Scope:** Turn 1 changed files only — callscope-core (schema, envelope, fingerprint, query, mermaid, lib) + acceptance fixture (parser lib + integration test) + workspace Cargo.toml / .gitignore. P4 (nightly indexer) and P7 (MCP server) do not exist yet and are reviewed only as consumers-to-be.

## Summary

The stable core is in good shape: it builds clean, all 43 core unit tests pass, the fixture builds and its 6 tests pass, and the `Envelope<T>` uncertainty contract (Q2/Q4/Q6/boundary) is genuinely wired through every built query, not just claimed. One real defect was found and empirically reproduced — `call_paths` over-reports truncation on a boundary case — plus three lower-severity risks, the most important being that the FNV-1a collision safeguard the schema comment promises is not implemented anywhere and must be designed into P4.

## Totals

- Critical: 0
- High: 0
- Medium: 2 (call_paths false-positive truncation; SymbolId collision has no detection)
- Low: 2 (call_paths can drop Q2 flag when truncated; Mermaid label escaping unverified against full v11 grammar)

All four are filed as issues under this Circle's `issues/`.

## Verification performed

- `cargo test -p callscope-core` → 43 passed, 0 failed.
- `cargo test` in `fixtures/workspace` → 4 unit (incl. `#[tokio::test]`) + 2 integration passed.
- Probe binary against callscope-core confirming the call_paths truncation defect (see finding 1).

## Findings by theme

### Uncertainty reporting (Q2/Q4) — the tender's load-bearing bar

**1. [Medium] `call_paths` reports `truncated=true` when path count == `max_paths`.**
`query.rs:369,:396`. `hit_cap = paths.len() >= max_paths` cannot tell "exactly max_paths paths exist" from "more exist, stopped early". Empirically: two real paths, `max_paths=2` → `data.len()=2, total=2, truncated=true`. The agent is told an exact answer is incomplete, and `total` equals what it received. Conservative direction (over-warns), so not a soundness hole, but it corrupts the Q4 signal on the common case where path count meets the cap. Fix: collect `max_paths+1`, report `truncated = len > max_paths`. Filed: `260726-1930_o_call-paths-false-positive-truncated.md`.

**2. [Low] `call_paths` flags computed only from returned paths.**
`query.rs:374-382`. Over-approximation and boundary flags are read off surviving paths after the cap, so if the dropped paths were the ones crossing a `Virtual`/boundary edge, Q2's `over_approximated` can be absent on a truncated answer. Mitigated by `truncated` being set. Fix: accumulate flags during the DFS. Filed: `260726-1930_o_call-paths-can-drop-over-approx-flag-when-truncated.md`.

Note (not filed): `affected_tests`/`reachability` set `over_approximated` if *any* virtual edge exists anywhere in the walked cone, even on a branch not leading to a returned symbol. This is coarse but conservative and matches the plan ("flags encountered along the walk"). Acceptable.

### Identity / collision safety

**3. [Medium] SymbolId FNV-1a collision has no detection; a collision silently merges symbols.**
`schema.rs:41-55` + loader `query.rs:137` (`by_id.insert` = last-write-wins). The schema comment claims the indexer's symbol table would catch a collision, but P4 is unbuilt and nothing enforces it. Probability at workspace scale is negligible (~1e-13 at a few thousand symbols), so it is not a practical hazard — the defect is an asserted-but-absent safeguard that must be built into P4 (fail-on-collision) or into `Graph::new` (assert unique ids) before P4 exists. Filed: `260726-1930_o_symbolid-collision-has-no-detection-silent-merge.md`.

### Mermaid v11 safety (C8)

**4. [Low] Label escaping proven only for the enumerated breakers; no render-based check.**
`mermaid.rs:291-296`. Node ids ARE safe by construction (`n<i>`/`b<i>`/`trunc_note` only — verified). Labels escape `& < > "` and never insert `\n`, which covers the documented v11 breakers, but nothing renders the output against a real v11 parser, and compiler-generated `{...}` names (if any survive P4's closure/async folding into `fq_path`) would land unescaped in a quoted label. Real-function paths are fine. Also: the `implementor_count` in the C8 envelope (`mermaid.rs:187-188`) counts virtual edges from kept nodes even when the edge was not drawn, so it can exceed the dashed edges shown — advisory, harmless. Filed: `260726-1930_o_mermaid-label-escaping-unverified-against-v11-grammar.md`.

## What was checked and found sound

- **Envelope end-to-end (Q2/Q4/Q6).** Uncertainty fields are `Option` + `skip_serializing_if`, so "no uncertainty" reads as absence, not null (envelope.rs:70-74; test at :136). Every built query sets `total` to the true pre-truncation count for C1/C2/C3/C5/C6/C7 (the one exception, C4's `total`, is finding 1). `stale` is correctly *not* set in the query layer and deferred to the MCP server (query.rs:30-33). No path silently drops staleness or over-approximation except the two call_paths cases above.
- **Reachability both directions.** Forward/backward walks are cycle-safe via an `enqueued` set; the full finite component is walked so `total` is exact; start excluded unless a cycle returns to it (query.rs:230-256). Tests cover both directions, cycles, and truncation.
- **Flag propagation.** `Virtual` edge → `over_approximated` with distinct-target count; foreign or absent target → `boundary_applies`, and a boundary target is treated as a non-expanded leaf (query.rs:180-196, :244-247). Both directions and the C7 merge are tested.
- **Fingerprint staleness.** Added / removed / changed and `Cargo.lock` drift all detected, with sorted+deduped output; determinism via `BTreeMap` (fingerprint.rs:102-126). The hash-all (not mtime-first) choice is justified against the fixed manifest schema and is exact. `civil_from_days` date conversion is correct against known epochs incl. a leap year. Minor documented edge cases (symlinked sources excluded; every `.rs` under root hashed regardless of workspace membership) are conservative — they never miss a real edit — so not filed.
- **Fixture coverage.** Genuinely exercises all four gaps and makes the guiding example answerable: gap 1 (generic `run_generic<T>`, reached by an in-crate test and a separate-crate integration test that never name the target), gap 2 (`run_dyn` + `&dyn Tokenizer`), gap 3 (Simple::tokenize's `.map(|t| normalize_token(t))` closure AND `tokenize_async`), gap 4 (`#[test]`, `#[tokio::test]`, and a real reachable `unsafe { from_raw_parts }`). The two load-bearing acceptance answers (generic-dispatch test + separate-crate integration test) are both present. No construct missing or misattributed.
- **Known FNV duplication** (`schema.rs` vs `fingerprint.rs`) is already filed (`260726-1914_o_...`); not re-filed. Cross-reference only.

## Recommended sequencing

- Before the acceptance harness (P11) leans on C4: fix finding 1 so `truncated` is trustworthy on `call_paths` — cheap change, and P11 will otherwise assert against a wrong signal.
- Before P4 (nightly indexer) is written: decide collision handling (finding 3) so the safeguard is designed in, not retrofitted.
- Findings 2 and 4 are cleanup — fold into the C4 fix and the P11 harness respectively.

None of the four blocks Turn 1 closure; they are correctness-polish and forward-looking guards, not release blockers for a stable-core-only turn.
