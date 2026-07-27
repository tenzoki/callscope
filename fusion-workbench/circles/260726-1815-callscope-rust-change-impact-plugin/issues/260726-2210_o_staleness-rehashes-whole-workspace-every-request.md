Staleness check re-hashes every source file on every MCP request
---
Every tool runs the staleness check first (`crates/callscope-mcp/src/tools.rs`,
`self.stale()?` in all eight tools), which calls
`IndexState::compute_stale` -> `fingerprint::diverged_files`
(`crates/callscope-mcp/src/state.rs:156-163`). The fingerprint is hash-all, not
mtime-first (noted in the Turn 1 review for `fingerprint.rs`), so each request
content-hashes every indexed `.rs` file plus `Cargo.lock`.
---
Severity: Low. Correctness is fine — this is what makes Q6 exact per request. The
concern is latency under interactive use: an agent working a change asks many
questions in a session, and each one re-hashes the whole workspace. On a large
target workspace that is repeated full-tree hashing per tool call.

The plan's own risk table (`planning/260726-1838_p_...:170`) anticipated a
"mtime-first fingerprint" precisely to avoid rehashing unchanged files; the
shipped fingerprint hashes all. Per-request invocation compounds it.

Fix direction: add the mtime-first fast path to the fingerprint (stat first,
hash only files whose mtime moved), or cache the last staleness result and
recompute only when a cheap mtime scan shows movement. The fix lives in
`callscope-core::fingerprint`; the per-request call site in the server is
correct and should stay.

Does NOT block P11 (fixture is tiny). Deferrable performance hardening.

Affects: callscope-core (fingerprint), callscope-mcp (per-request call). Related
to the accepted hash-all choice from P3.

---
Reconciliation 260726-2316: still OPEN. Verified fingerprint.rs still ships hash-all (module docs "Staleness strategy: hash-all, not mtime-first"); the mtime-first fast path is not implemented. This is the deferred Option 2 of decision 260726-1914 (now _i_ for the hash-all v1 answer). Low-severity performance follow-up; does not block closure.
