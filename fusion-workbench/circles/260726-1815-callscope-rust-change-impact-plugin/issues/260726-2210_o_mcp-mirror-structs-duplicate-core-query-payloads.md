callscope-mcp maintains mirror structs because core's query payloads do not derive Serialize
---
`callscope-core`'s query payloads derive no `Serialize`:
`crates/callscope-core/src/query.rs:59` (`DirectCalls`), `:69` (`CallPath`),
`:75` (`Impact`). Because it cannot edit core, `callscope-mcp` defines
serializable mirror structs `DirectCallsOut` / `CallPathOut` / `ImpactOut` and
remaps each answer onto them: `crates/callscope-mcp/src/state.rs:43-63`,
plus `remap_envelope` at `:69-78` and the per-tool remaps at `:206-265`.
---
Severity: Low. This is a single-source-of-truth smell, not a live defect. The
mirror is currently *safe*: each remap destructures the core payload with an
exhaustive struct pattern (`let DirectCalls { callers, callees } = ...`), so a
new field on a core payload breaks compilation rather than being silently
dropped — the failure mode the mirror could have introduced is caught by the
compiler. `remap_envelope` copies all six `Envelope` fields (`data`, `stale`,
`over_approximated`, `truncated`, `total`, `boundary_applies`), so no uncertainty
flag is lost in the remap (verified against `envelope.rs:66-83`).

The cost is maintenance: three parallel type families that a reader must prove
are equivalent, and every new query payload needs a matching mirror.

Fix direction: derive `Serialize` (and `Deserialize` if useful) on
`DirectCalls`, `CallPath`, and `Impact` in `callscope-core`, then delete the
three `*Out` mirrors and the manual remaps, returning the core payloads directly
through the envelope. One type per payload, owned by core.

Affects: callscope-core (query payload derives), callscope-mcp (state.rs mirrors).
Flagged in the review's latent-design section.

---
Reconciliation 260726-2316: still OPEN. Verified callscope-core query payloads (DirectCalls/CallPath/Impact, query.rs:58-75) still derive no Serialize, so the callscope-mcp mirror structs remain necessary. Confirmed still safe (exhaustive destructuring). Low-severity SSOT smell; deferrable. Does not block closure.
