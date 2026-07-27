SymbolId FNV-1a collisions have no detection anywhere; a collision silently merges two symbols — the claimed safeguard lives only in the unbuilt indexer

---
`SymbolId` is a 64-bit FNV-1a hash of `fq_path` (crates/callscope-core/src/schema.rs:41-55).
The doc comment (schema.rs:36-39) argues a collision "would be caught by the
indexer's own symbol table (two distinct fq_paths hashing equal is detectable
there)." That safeguard does not exist: `callscope-index` (P4) is unbuilt, and
nothing in the current code checks for it.

Where the failure lands: `Graph::new` (query.rs:137) builds `by_id` with
`by_id.insert(sym.id, sym)` — last write wins. If two distinct `fq_path`s hash
equal, one symbol is silently discarded and every edge referencing that id folds
onto the survivor. Reachability, affected-tests, and impact answers would then be
silently wrong, with no flag — a direct Q1/Q2 violation (a silent best guess).

Probability at workspace scale is negligible: by the birthday bound a 64-bit
space needs ~5e9 symbols for a 50% chance; at a few thousand symbols it is ~1e-13.
So this is not a practical hazard today. The defect is that the code *claims* a
detection safeguard it does not implement, and the natural place to implement it
(P4) has not been written yet — so it is easy to ship P4 without it and leave the
comment lying.

Severity: Medium — not because a collision is likely, but because the mitigation
is asserted-but-absent and must be designed into P4 before that crate exists,
not retrofitted after.

---
Fix direction (for P4, callscope-index): when building the index, detect two
distinct `fq_path`s producing the same `SymbolId` and fail the index build (or
disambiguate) rather than emitting a graph that `Graph::new` will silently merge.
Alternatively, `Graph::new` could assert no id collision among the loaded symbols
and surface it rather than last-write-wins. Until then, soften the schema.rs
comment so it does not claim a safeguard that is not present.

Affects: callscope-core (schema, query loader) and callscope-index (P4).

---
Resolved (P4, callscope-index): fail-on-collision is implemented and tested.
`merge()` (crates/callscope-index/src/main.rs) detects two distinct `fq_path`s
carrying the same `SymbolId` and aborts the index build with a diagnostic that
names both paths and the shared id, rather than letting `Graph::new`'s
last-write-wins silently merge them. The per-crate `Builder` was also changed to
key its symbol table by `fq_path` (not by `SymbolId`) so a within-crate
collision survives into the emitted fragment and reaches that same check — an
id-keyed map would have dropped one symbol before `merge` could see the
conflict. Two unit tests cover it: `merge_fails_on_symbolid_collision` (asserts
the build fails with both paths named) and `merge_unions_same_symbol_across_fragments`
(asserts an identical id+path across fragments is a normal characteristics-union
merge, not a false-positive collision). `cargo test -p callscope-index`: 2 passed.
The schema.rs comment (schema.rs:36-39) is now accurate — the asserted
safeguard exists in P4.
