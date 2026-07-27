# How should the `dyn` over-approximation represent a GENERIC implementor?

---
**Domain:** code
**Status:** implemented
**Filed by:** coder
**Cross-references:** issues/260726-2210_c_emit-virtual-empty-genericargs-drops-generic-implementors.md; issues/260726-2210_c_emit-virtual-only-enumerates-local-crate-impls.md; reviews/260726-2210-coderev-turn2-index-and-mcp.md

---

## Question

Closing the generic-implementor gap (issue `260726-2210_..._empty-genericargs...`)
required deciding *how* a `dyn Trait` over-approximation represents a generic
implementor such as `impl<T: Tokenizer> Tokenizer for Wrapper<T>`. A `&dyn
Tokenizer` call site has no concrete `T`, so there is no single monomorphized
`<Wrapper<Concrete> as Tokenizer>::tokenize` to point the widened edge at. On the
pinned nightly, `Instance::resolve(impl_fn, &GenericArgs(vec![]))` does not even
fail cleanly — it returns a *polymorphic* instance whose `body()` then panics
(`rustc_type_ir/src/binder.rs`: "has parameters, but no args were provided in
instantiate"). So the old code did not merely drop the implementor; on a real
generic fixture it aborted the whole crate compilation.

What symbol should the over-approximation edge target, and how honest is the
reachability computed through it?

## Options

1. **One polymorphic method node, walked un-monomorphized (chosen).** Name the
   implementor by its generic method path (`<parser::Wrapper<T> as
   parser::Tokenizer>::tokenize`), flag the symbol `generic`, and walk its body
   ONCE via `FnDef::body()` (the item-level `mir_body`, which does not
   monomorphize and does not panic). The `dyn` edge points at that node.
   - Pros: the implementor is included (never silently dropped); its body's
     concrete calls resolve (`Wrapper<T>::tokenize` genuinely reaches
     `normalize_token` through its folded closure); the residual imprecision is
     visible (the node is flagged `generic`, the edge is `Virtual`, so the
     answer is `over_approximated`). No compiler-internal MIR extraction path is
     introduced — it reuses the existing `walk_body` machinery.
   - Cons: it represents the implementor by ONE representative body, not by each
     concrete `Wrapper<X>` monomorphization. Inner calls that are themselves
     generic (`self.0.tokenize()` = `<T as Tokenizer>::tokenize`) become a
     further `dyn` over-approximation to all implementors, rather than the exact
     `X`. `implementor_count` counts the generic impl as one, regardless of how
     many concrete `Wrapper<X>` the workspace instantiates.
2. **Enumerate concrete monomorphizations.** Point the edge at each walked
   `<Wrapper<X> as Tokenizer>::tokenize` symbol found in the workspace.
   - Pros: per-instantiation precision.
   - Cons: `rustc_public` on this nightly exposes no clean way to enumerate the
     vtable/mono instances of a generic impl from the `dyn` site's crate; the
     instances are scattered across whichever crates coerce them, and matching a
     generic def to its monomorphized symbol names is brittle string surgery.
     Not recoverable cleanly here.
3. **Bare flagged node, body not walked.** Emit the generic node but do not walk
   its body.
   - Pros: simplest.
   - Cons: strictly worse than option 1 — the implementor would not reach
     `normalize_token`, so `affected_tests` would miss a real path, and the node
     would be an information-free stub.

## Constraints

- Honesty (tender Q2): a generic implementor that could fire MUST be visible in
  the widened set; residual imprecision must be *flagged*, never silent. Do not
  fake an exact claim.
- Scope: `crates/callscope-index/**` only — no changes to the query layer or the
  envelope, so `implementor_count` is derived from distinct virtual-edge targets
  as-is.

## Recommendation

Option 1, implemented. It is the best honest over-approximation recoverable from
`rustc_public` on the pinned nightly: the implementor is included and genuinely
reaches the target, and the two visible flags (`generic` on the symbol,
`over_approximated`/`Virtual` on the answer) mark the residual precisely.

**Residual, stated precisely:** a generic implementor is folded into the widened
set as a single polymorphic method node. Reachability *through* it is computed
over the un-monomorphized body once; its own generic sub-calls widen again; and
it counts as one implementor irrespective of the number of concrete `Wrapper<X>`
instantiations. This is a superset (it never omits a path that could fire), so it
stays sound for `affected_tests`/`impact`; it is not per-instantiation exact, and
the `generic` flag is the signal that it is not.

---
Answered: crates/callscope-index/src/graph_build.rs:collect_impls + walk_generic_method, crates/callscope-index/src/main.rs:merge (dyn join) — generic implementor walked once via FnDef::body(), flagged generic, widened as a Virtual edge.
Implemented: ea1eae1 (FIX-DYN — merge-time dyn join emits one Virtual edge per implementor across all workspace members incl. generic + cross-crate). Acceptance DYN check confirms end-to-end: run_dyn widens to 4 implementors (Simple, Fancy, Wrapper<T> generic, ext_tokenizer::Shouty cross-crate), implementor_count=4. Residual (single polymorphic node, not per-instantiation exact) stands as a documented, flagged over-approximation — sound superset, not a contradiction.
