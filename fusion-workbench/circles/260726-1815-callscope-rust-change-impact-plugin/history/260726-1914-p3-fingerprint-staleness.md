# P3 — Fingerprint and staleness module in callscope-core

**Date:** 260726-1914
**Agent:** coder
**Status:** Complete
**Plan:** planning/260726-1838_p_callscope-implementation.md step 3
**Task:** P3 in tasklist.md

## What was implemented

Filled the comment-only stub `crates/callscope-core/src/fingerprint.rs` with the
workspace source fingerprint and staleness check (Q5 repeatable, Q6 detectable
staleness). Public API, single-sourced for both `callscope-index` (P4) and
`callscope-mcp` (P7):

- `fingerprint_workspace(root, toolchain) -> io::Result<Manifest>` — walks the
  tree for every `.rs` file, hashes each, hashes `Cargo.lock`, and stamps
  toolchain + `indexed_at` (RFC-3339 UTC). Produces the P2 `Manifest`
  (`file_hashes: BTreeMap<String,String>` keyed by workspace-relative path with
  `/` separators, `cargo_lock_hash`, `toolchain`, `indexed_at`, `schema_version`).
- `diverged_files(manifest, root) -> io::Result<Vec<String>>` — returns the
  changed / added / removed `.rs` files, plus `Cargo.lock` when its hash moved.
  Sorted, deduped. Empty means up to date. Populates `StaleInfo { diverged_files }`.

Both entry points share one private `hash_all_rs` walker, so the two consuming
crates cannot drift on what "the same workspace state" means. The walk skips
`target/`, `.git/`, and any dotted directory, and ignores symlinks (no cycles).

No global state; the only impurity is the clock read for `indexed_at`, isolated
in `now_rfc3339` over a pure `unix_secs_to_rfc3339`.

## Hash choice

FNV-1a 64-bit, rendered as 16 hex digits. Reuses the algorithm the codebase
already uses for `SymbolId` content-addressing (schema.rs). Reasons: one hashing
algorithm across the crate; fixed and dependency-free, so it is stable across
platforms and toolchains (unlike std SipHash, which would report false staleness
on unchanged code after a toolchain bump); crypto strength is not needed for
change detection. No hashing crate added, so `callscope-core/Cargo.toml` was
left untouched.

## Staleness strategy (deviation from plan, recorded)

The plan sketched a mtime-first fast path. That needs a stored mtime per file to
compare against, and the P2 `Manifest` records content hashes only — editing the
schema was out of P3 scope. So the check hashes all `.rs` files and diffs. Exact
and fast at workspace scale. Recorded in
decisions/260726-1914_a_staleness-hash-all-vs-mtime-first.md.

## Tests

9 new unit tests in the module (16 total in the crate with P2's). Cover:
fingerprint stable across two runs on identical content; a content edit flips the
hash and staleness reports exactly that file; add detected; remove detected;
Cargo.lock change detected; no divergence on an identical workspace; target/ and
dotted dirs excluded; FNV primitive distinctness/repeatability; RFC-3339
conversion against known epochs (incl. a leap-year crossing). Tests use a
std-only self-cleaning temp-dir guard rather than the `tempfile` crate, so they
run offline and add no dev-dependency.

## Verification

Command: `cargo test -p callscope-core`
Result: 16 passed, 0 failed (9 new + 7 from P2). Doc-tests: 0.

## Files changed

- crates/callscope-core/src/fingerprint.rs (implemented; was a stub)

## Tracking updated

- tasklist.md: P3 → [x]
- plan step 3 → [DONE]
- Filed decision: decisions/260726-1914_a_staleness-hash-all-vs-mtime-first.md
- Filed issue: issues/260726-1914_o_fnv1a-duplicated-between-schema-and-fingerprint.md

## Notes for downstream

- P4 (callscope-index) writes the manifest via `fingerprint_workspace`; P7
  (callscope-mcp) checks staleness via `diverged_files`. One implementation, no
  second copy.
- `callscope-core/Cargo.toml` unchanged — no runtime or dev dependency added.
- Voice profile: chat-voice-en.yaml loaded and applied to this report.
