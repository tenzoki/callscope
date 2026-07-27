call_paths reports `truncated=true` when the path count exactly equals `max_paths` (false-positive uncertainty)

---
`Graph::call_paths` (crates/callscope-core/src/query.rs:369, :396) sets
`truncated` from `hit_cap = paths.len() >= max_paths`. When the number of real
paths is exactly `max_paths`, `paths.len() == max_paths`, so `hit_cap` is true
and the answer is flagged truncated even though nothing was dropped. This is a
Q4 defect: `truncated` must mean "a size bound cut the result short", and here
it fires on a complete result.

Empirically confirmed (probe against callscope-core, 260726):
graph a->b->d and a->c->d (exactly two paths a..d), `call_paths(a, d, depth=10,
max_paths=2)` returns `data.len()=2`, `total=2`, `truncated=true`. The agent is
told the path list is incomplete while receiving every path, and `total`
equals what it received — a contradictory signal.

Severity: Medium. Direction is conservative (over-warns, never under-warns), so
it is not a soundness hole, but it pollutes the "visible uncertainty" signal on
a common boundary case (path count == cap) that the tender's Q2/Q4 make
load-bearing.

Related, same root: for C4 alone, `total` is only the returned-path count
(query.rs:393), not the true number of paths — unlike C1/C3/C5/C6 where `total`
is the full pre-truncation count. When genuinely capped, the agent cannot learn
how many paths really exist. Enumerating all paths just to count them can blow
up, so a true total may be infeasible; at minimum the false-positive above
should be fixed so `truncated` is trustworthy.

---
Fix direction: distinguish "found exactly max_paths and stopped" from "more
exist". Standard approach: let the DFS collect up to `max_paths + 1` paths, then
report `truncated = paths.len() > max_paths` and return the first `max_paths`.
That makes `truncated` exact for the count cap. The depth-cut branch
(query.rs:421-426) is a separate, also-conservative over-report (it sets
`depth_cut` when the node has any onward edge, even if every extension would
revisit an on-path node or a boundary leaf) — worth tightening in the same pass
but lower priority.

Affects: callscope-core query engine (C4). Shared by callscope-mcp (P7) once it
wraps this tool.

---
Resolved: 260726-2056. Reworked `Graph::call_paths` (crates/callscope-core/src/query.rs) to enumerate up to `max_paths + 1` paths via a probe path, then report `truncated = collected.len() > max_paths` (the "more may exist" signal) and return only the first `max_paths` after sorting. A result with exactly `max_paths` real paths now reports `truncated = false`. `total` is documented as the RETURNED path count (a true full count can be exponential); `truncated` stays the independent "more may exist" signal, also set when a branch was cut at `max_depth`. The two distinct semantics are spelled out in the function doc comment. New test `call_paths_exactly_max_paths_is_not_truncated` locks the boundary case; `call_paths_over_max_paths_truncates_and_returns_exactly_cap` locks the over-cap case. `cargo test -p callscope-core` green (46 passed).
