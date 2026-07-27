FNV-1a implementation duplicated between schema.rs and fingerprint.rs
---
The FNV-1a 64-bit algorithm (offset basis, prime, and the xor/multiply loop) now
exists in two places in `callscope-core`: `SymbolId::from_fq_path` in
`crates/callscope-core/src/schema.rs` and `fnv1a_64_hex` in
`crates/callscope-core/src/fingerprint.rs`. Both are content-addressing hashes
serving the same purpose, and both must stay byte-for-byte identical to keep the
crate on one hashing algorithm.
---
Low severity — the constants are the fixed FNV-1a standard, so the two copies
cannot silently disagree, and the algorithm is a handful of lines. But it is a
single-source-of-truth smell. The duplication was accepted deliberately in P3:
P3's scope forbade editing `schema.rs`, so consolidating was not possible then.

Fix when `schema.rs` can be touched: extract one `fn fnv1a_64(bytes: &[u8]) -> u64`
(e.g. a small `hash` module or a `pub(crate)` helper), have `SymbolId::from_fq_path`
call it over the path bytes, and have `fingerprint::fnv1a_64_hex` call it and
hex-format the result. One algorithm, one place.

---
Reconciliation 260726-2316: still OPEN. Verified schema.rs still carries the FNV loop (`wrapping_mul(PRIME)` at schema.rs:51); no consolidation landed across the 13 session commits. Low-severity SSOT smell, does not block Circle closure. Fix still gated on a task permitted to edit schema.rs.
