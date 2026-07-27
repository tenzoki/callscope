call_paths computes Q2 over-approximation / boundary flags only from the returned paths, so truncation can drop them

---
`Graph::call_paths` (crates/callscope-core/src/query.rs:374-382) accumulates
`WalkFlags` by iterating over the *returned* paths' edges only. Every other query
notes flags while walking the full structure, but call_paths caps the path set
first (max_paths / max_depth) and then reads flags off what survived.

Consequence: if the paths that cross a `Virtual` (dyn-dispatch) edge or a
boundary edge are exactly the ones dropped by the `max_paths` cap, the returned
envelope has `over_approximated = None` / `boundary_applies = false` even though
the true (uncapped) answer would carry them. Q2 says over-approximation must be
visible; here it can silently vanish on a truncated answer.

Mitigating factor: `truncated` is set on such answers (though see the companion
issue on false-positive truncation), so the agent is told the result is
incomplete. The returned paths themselves are correctly flagged. So the risk is
"incomplete answer under-states its uncertainty", not "complete answer is wrong".

Severity: Low.

---
Fix direction: accumulate `WalkFlags` during the DFS as edges are traversed
(the same place cycles/depth are handled), not in a post-hoc pass over survivors.
Then flags reflect every edge the enumeration touched, independent of the cap.

Affects: callscope-core query engine (C4).

---
Resolved: 260726-2056. Moved WalkFlags accumulation into the `dfs_paths` traversal (crates/callscope-core/src/query.rs): `note_edge` is now called on every edge the DFS traverses, replacing the post-hoc pass that read flags off the surviving paths only. A path dropped by the `max_paths` cap now still contributes its Virtual/boundary uncertainty, so over-approximation and boundary flags are never silently lost on a truncated answer. Since the enumeration collects up to `max_paths + 1` paths, flags cover every edge visited within that bound; the residual limitation (edges reachable only along paths beyond the collect cap are not visited) is documented honestly in the function doc comment, and such answers are marked `truncated`. New test `call_paths_truncated_still_flags_virtual_edge_on_dropped_path` locks the fix: with `max_paths=1`, the dropped path carries the Virtual edge and the returned envelope still reports `over_approximated`. `cargo test -p callscope-core` green (46 passed). Removed now-dead `find_edge` helper (was only used by the deleted post-hoc pass).
