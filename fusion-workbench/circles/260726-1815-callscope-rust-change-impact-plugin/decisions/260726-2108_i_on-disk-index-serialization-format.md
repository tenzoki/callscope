# What serialization format do `index.bin` and `manifest.json` use on disk?

---
**Domain:** code
**Status:** answered
**Filed by:** coder
**Cross-references:** crates/callscope-mcp/src/state.rs (reader), crates/callscope-index (writer, P4 — not yet built), crates/callscope-core/src/schema.rs (the serde types)

---

## Question

`callscope-mcp` (P7) must read `.callscope/index.bin` and `.callscope/manifest.json`
from disk, but `callscope-index` (P4, the writer) is being built in parallel and does
not exist yet. The reader therefore has no writer to constrain its choice of wire
format, yet the two must agree byte-for-byte or the index will not load. The plan names
the files `index.bin` (suggesting binary) and `manifest.json` (suggesting JSON) but
never fixes the actual `serde` format. A format must be pinned now so P4 can match it.

## Options

1. **`serde_json` for both files.** — Read/write via `serde_json::to_vec` /
   `from_slice`.
   - Pros: no new dependency (`callscope-core` already depends on `serde_json`, and its
     schema tests already round-trip the `Index` through `serde_json`); human-inspectable
     artifacts, so a stale or malformed index is debuggable by eye; one format for both
     files.
   - Cons: `index.bin` is not actually binary despite the extension; larger on disk than
     a packed binary encoding.
2. **`bincode` (or `postcard`) for `index.bin`, JSON for the manifest.** — A compact
   binary encoding for the graph.
   - Pros: smaller, faster to parse at scale; the `.bin` extension becomes honest.
   - Cons: adds a dependency to both `callscope-core`/`callscope-index` and
     `callscope-mcp`; opaque artifact; premature optimization at v1 workspace scale.

## Constraints

- The reader (`callscope-mcp`) and writer (`callscope-index`) MUST use the identical
  format and the identical `serde` type definitions from `callscope-core::schema`.
- The scope of P7 forbids editing `callscope-core` and `callscope-index`, so the reader
  could only pick a format, not enforce it on the writer — hence this record, so P4
  adopts the same choice deliberately rather than by accident.

## Recommendation

Option 1 — `serde_json` for both. It needs no new dependency, matches how
`callscope-core`'s own tests already serialize an `Index`, and keeps both artifacts
inspectable while the tool is young. The `.bin` name is kept for continuity with the
plan but carries JSON bytes in v1. If index size or parse time ever matters at a much
larger workspace scale, switching to a binary `serde` encoding is a localized change to
the two files' read/write calls (the schema types are unchanged) and can supersede this
record then.

---
Answered: crates/callscope-mcp/src/state.rs:118 (IndexState::load) — v1 reads both `index.bin` and `manifest.json` as JSON via `serde_json::from_slice`. P4's `callscope-index` MUST write both with `serde_json` and the same `callscope-core::schema` types.
Implemented:
Deferred:
Superseded by:
Implemented: ec14c5b (callscope-index writes serde_json Index/Manifest) + ccdace0 (callscope-mcp reads them via serde_json into core types); fixture index loads OK.
