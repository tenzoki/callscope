# Staleness check: hash every `.rs` file, or stat mtimes first?

---
**Domain:** code
**Status:** implemented
**Filed by:** coder
**Cross-references:** planning/260726-1838_p_callscope-implementation.md step 3 and Risks table ("mtime-first fingerprint avoids rehashing unchanged files"); crates/callscope-core/src/fingerprint.rs; crates/callscope-core/src/schema.rs (Manifest)

---

## Question

The plan's step 3 and its risk table both describe a mtime-first staleness fast
path: stat each file's mtime, hash only the ones whose mtime moved. Implementing
P3 surfaced a conflict: that fast path needs a *stored* mtime per file to compare
the current mtime against, and the `Manifest` schema (fixed by P2, out of scope
to edit in P3) records content hashes only — no per-file mtimes. So the planned
optimisation cannot be built without a schema change. What does P3 do instead?

## Options

1. **Hash-all** — re-hash every `.rs` file on each staleness check and diff the
   result against the stored hashes.
   - Pros: exact (never misses a `touch`-only or within-mtime-granularity edit);
     reuses the same per-file hashing as the fingerprint, so index and mcp share
     one notion of "same workspace state"; no schema change.
   - Cons: O(total source bytes) per check rather than O(changed files).
2. **Extend the manifest with per-file mtimes, then stat-first** — the plan's
   original intent.
   - Pros: matches the plan; cheaper on large workspaces.
   - Cons: a `Manifest` schema change, explicitly outside P3's scope; mtime is a
     weaker signal (a restore or checkout can reset mtime without a content
     change, and vice versa), so it still needs a hash fallback to be exact.

## Constraints

- P3 may not edit `schema.rs` (the `Manifest` type). 
- The fingerprint must be exact for change detection — a missed edit means a
  stale index served as fresh (a Q6 failure).

## Recommendation

Option 1, taken. The task itself permits it ("or hash all if that is simpler and
still fast for a small workspace"). For a workspace of callscope's scale,
re-hashing is trivially fast and exact. If re-index cost ever matters at much
larger scale, revisit by growing the manifest with per-file mtimes (Option 2) as
a follow-up — that is the schema change this decision defers, not rejects.

---
Answered: crates/callscope-core/src/fingerprint.rs (module docs "Staleness strategy: hash-all, not mtime-first"; `diverged_files`) — hash-all chosen because the P2 Manifest carries no stored mtime to compare against; exact and fast at workspace scale.
Implemented: e274a71 (callscope-core fingerprint module ships hash-all `diverged_files`; verified still hash-all at reconciliation 260726-2316). Q5/Q6 pass in acceptance 19/19. The deferred Option 2 (manifest per-file mtimes + stat-first) remains tracked as open follow-up issue 260726-2210_o_staleness-rehashes-whole-workspace-every-request.md — that is a separate future decision, not a reopening of this one.
