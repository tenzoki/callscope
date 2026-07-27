# Fix C4 call_paths: false-positive truncation + dropped uncertainty flags

**Agent:** coder
**Session:** 260726-2056
**Status:** Complete
**Circle:** 260726-1815-callscope-rust-change-impact-plugin
**Scope:** crates/callscope-core/src/query.rs only (public signature of `call_paths` unchanged)

## What was done

Fixed two coderev findings in the C4 `call_paths` implementation.

### Issue 1 — false-positive `truncated` (Medium)
`260726-1930_c_call-paths-false-positive-truncated.md`

Old code set `hit_cap = paths.len() >= max_paths`, so a complete result of exactly
`max_paths` paths was flagged truncated. Reworked the DFS to enumerate up to
`max_paths + 1` paths (a probe path), then report `truncated = collected.len() > max_paths`
and return only the first `max_paths` after the deterministic sort. `total` is now
documented as the RETURNED path count (a true full count can be exponential); `truncated`
is the independent "more may exist" signal, still also set when a branch was cut at
`max_depth`. The two semantics are spelled out in the function doc comment.

### Issue 2 — uncertainty flags dropped on truncation (Low)
`260726-1930_c_call-paths-can-drop-over-approx-flag-when-truncated.md`

Old code read `WalkFlags` in a post-hoc pass over the *returned* paths only, so a Virtual
or boundary edge on a dropped path lost its flag. Moved flag accumulation into `dfs_paths`:
`note_edge` is now called on every edge the DFS traverses, so dropped paths still contribute
their uncertainty. Residual limitation (edges reachable only beyond the `max_paths + 1`
collect cap are not visited) is documented in the doc comment; such answers are marked
`truncated`. Removed the now-dead `find_edge` helper (only the deleted post-hoc pass used it).

## Files changed

- `crates/callscope-core/src/query.rs`
  - `call_paths`: rewritten doc comment (total vs truncated semantics; flag coverage note),
    probe-path enumeration, `truncated = collected.len() > max_paths || depth_cut`, sort +
    truncate to `max_paths`, flags carried in from the DFS.
  - `dfs_paths`: added `flags: &mut WalkFlags` param, `collect_cap` param, `note_edge` on
    each traversed edge, cap comparison uses `>= collect_cap`.
  - Removed dead `find_edge`.
  - Added 3 unit tests: `call_paths_exactly_max_paths_is_not_truncated`,
    `call_paths_over_max_paths_truncates_and_returns_exactly_cap`,
    `call_paths_truncated_still_flags_virtual_edge_on_dropped_path`.

## Verification

- `cargo test -p callscope-core` → **46 passed; 0 failed** (43 pre-existing + 3 new).
- `cargo build -p callscope-core --all-targets` → no warnings, no errors.
- clippy unavailable on the pinned nightly toolchain (not installed); build used instead.

## Tracking updates

- Both issue files appended `Resolved:` notes and renamed `_o_` → `_c_`.
- tasklist.md not updated — it tracks plan tasks P1–P11, not individual review findings.
