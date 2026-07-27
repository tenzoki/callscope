# P2 — Index schema and output envelope in callscope-core

**Date:** 260726-1904
**Agent:** coder
**Status:** Complete
**Plan:** circles/260726-1815-callscope-rust-change-impact-plugin/planning/260726-1838_p_callscope-implementation.md (step 2)

## What was implemented

Filled the two comment-only stubs in `callscope-core` per the plan's Data Structures section.

### schema.rs — the serde-serializable call graph
- `SymbolId(u64)` newtype, `SCHEMA_VERSION` const.
- `Span { file, line_start, line_end }`, `Characteristics { test, public, is_async, generic, foreign, uses_unsafe }`.
- `Symbol { id, fq_path, crate_name, span, characteristics }`.
- `EdgeKind { Static, Virtual }`, `Edge { from, to, kind }`.
- `Index { schema_version, symbols, edges }` (+ `Index::empty()`), `Manifest { schema_version, toolchain, file_hashes: BTreeMap, cargo_lock_hash, indexed_at }`.

### envelope.rs — the one output shape
- `Envelope<T> { data, stale, over_approximated, truncated, total, boundary_applies }`.
- `StaleInfo { diverged_files }`, `Reason` enum (`#[non_exhaustive]`, `DynDispatch { trait_path, implementor_count }`).
- Constructor `Envelope::exact` + builder setters (`with_total`, `with_stale`, `with_over_approximation`, `with_truncated`, `with_boundary`); `StaleInfo::new`, `Reason::dyn_dispatch`.
- `Option` uncertainty fields carry `#[serde(default, skip_serializing_if = "Option::is_none")]` so absent uncertainty is omitted from JSON, not serialized as null.

### lib.rs
- Added crate-root `pub use` re-exports for the schema and envelope types.

## Choices made (where the plan left it open)

**SymbolId representation: 64-bit FNV-1a hash of the fully-qualified path**, not a positional index.
- Stability requirement: a caller must be able to reference a symbol across re-indexing of unchanged code. A positional `u32` breaks the moment any symbol is added/removed/reordered — every later index shifts. Content-addressing by `fq_path` stays fixed while the path is unchanged.
- FNV-1a over the std SipHash: SipHash output is not guaranteed stable across Rust versions/platforms, so an id could drift under a toolchain bump with no code change. FNV-1a is fixed and dependency-free.
- `u64` over a hex string: keeps the id `Copy` and compact for edge endpoints and map keys. Collision risk negligible at workspace scale; documented in the type doc.

## Verification

Command: `cargo test -p callscope-core`
Result: build clean (no warnings), 7 unit tests pass, 0 doc-tests.
- schema: SymbolId stability, SymbolId distinctness, `Index` serde round-trip, `Manifest` serde round-trip + BTreeMap key-order determinism.
- envelope: `exact` omits uncertainty fields, uncertainty fields present when set, `Envelope` serde round-trip.

## Scope adherence

Touched only `crates/callscope-core/src/schema.rs`, `envelope.rs`, `lib.rs`. No new deps needed (serde + serde_json already wired by P1). fingerprint/query/mermaid left as stubs. Root Cargo.toml, other crates, and the fixture untouched. No commit made — orchestrator commits.

## Files changed
- crates/callscope-core/src/schema.rs
- crates/callscope-core/src/envelope.rs
- crates/callscope-core/src/lib.rs
