# Should `boundary_applies` fire on ordinary std/core calls, or only on user-relevant dependency crossings?

---
**Domain:** code
**Status:** open
**Filed by:** coderev
**Cross-references:** crates/callscope-index/src/graph_build.rs:247-260 (handle_call static arm), crates/callscope-core/src/query.rs:180-185 (is_boundary_target), README.md:122-125 (boundary honesty claim), problem.md:60 (§7 boundary)

---

## Question

The indexer emits a call edge for every statically resolved callee that is an
`Item` instance, regardless of which crate it lives in
(`graph_build.rs:247-260`): the locality check only gates whether the callee is
*walked*, not whether an edge is *emitted*. So a call to `Vec::push`, `String::from`,
any `std`/`core`/`alloc` function produces an edge whose target id is absent from
every workspace fragment. At query time `is_boundary_target`
(`query.rs:180-185`) treats an absent target as a boundary crossing, so
`boundary_applies` is set.

Consequence: on any **forward** walk (C2 callees, C3 forward reachability, C6
reachable_unsafe) over real code, nearly every function calls into std, so
`boundary_applies` is true almost always. A flag that is true on nearly every
answer carries little signal. (Backward walks — affected_tests, impact — are
unaffected: they note `edge.to`, which on an incoming edge is the workspace node,
never the std leaf. Verified in `query.rs::note_edge` + `reachable_set`.)

This must be decided now because it directly affects the P11 acceptance harness:
any P11 assertion of the form "a pure-workspace forward answer has
`boundary_applies == false`" will fail against the real fixture index if the
walked functions call std at all. P11 should either avoid that assertion shape or
the boundary semantics should be narrowed first.

## Options

1. **Keep as-is (all non-workspace crates are the boundary, std included).**
   - Pros: literally honest — the walk did reach the edge of the user's crates;
     matches the plan's "mark any edge crossing into a third-party crate."
   - Cons: `boundary_applies` is near-constant true on forward walks, so it stops
     discriminating the case §7 actually cares about (a call chain that continues
     *through* a dependency, e.g. a callback handed to a framework). P11 forward
     assertions must account for it.
2. **Do not emit edges to std/core/alloc (or to any crate) as boundary; reserve the flag for dependency crossings that could continue the chain.**
   - Pros: `boundary_applies` regains signal — true only when the answer is
     genuinely incomplete past a dependency callback, which is the §7 intent.
   - Cons: needs a rule for "which crates count"; a leaf std call is arguably a
     real (if trivial) boundary, so this narrows honesty slightly.
3. **Distinguish "leaf external call" from "chain continues past a dependency" with two signals** (e.g. keep emitting the edge but only set `boundary_applies` when the external callee itself takes a callback / fn-pointer / trait-object argument that could re-enter workspace code).
   - Pros: most precise; matches the framework-callback example in §7 exactly.
   - Cons: most work; requires signature inspection at index time.

## Constraints

- Whatever is chosen, §7 requires that every answer whose completeness the
  boundary affects still states it. Narrowing must not hide a real incompleteness.
- The v1 workspace-boundary decision stands; this is only about how the flag is
  computed, not about following chains past the boundary.

## Recommendation

Option 2 for v1 (narrow the flag so it means "the walk reached a dependency the
tool does not descend into," not "the walk touched std"), with option 3 recorded
as the v2 refinement. At minimum, resolve this before P11 writes any
`boundary_applies` assertion on a forward walk.

---
Answered:
Implemented:
Deferred:
Superseded by:
