Mermaid label safety is proven only for the enumerated breakers (&, <, >, ", \n); no render-based verification exists, and compiler-generated names could carry other special characters

---
`escape_label` (crates/callscope-core/src/mermaid.rs:291-296) escapes `&`, `<`,
`>`, `"` and inserts no `\n`. Node ids are the generated `n<i>`/`b<i>`/`trunc_note`
forms, which ARE safe by construction — that part of the v11 claim holds.

The label claim is weaker than "by construction": labels are safe for the four
characters enumerated, but nothing verifies safety against the rest of the strict
v11 flowchart grammar. Rust `fq_path`s for ordinary functions will not contain
`{`, `}`, backticks, or `%`, so real symbols are fine. The gap is compiler-
generated names: if any `{{closure}}` / `{async_block}` / `{{constant}}`-style
synthetic name reaches a symbol's `fq_path` (the indexer is supposed to fold
closures/async into their parents, but that is P4's job and not yet verified),
its `{`/`}` would land unescaped inside a quoted label. Whether v11 tolerates
that inside `["..."]` is not tested here.

Severity: Low. Inference, not verified: I did not render the output against a
real Mermaid v11 parser.

---
Fix direction: (1) add a rendering/parse check to the acceptance harness (P11) so
"v11-safe" is demonstrated, not asserted; (2) consider escaping or stripping
`{`/`}`/backtick in `escape_label` defensively; (3) once P4 lands, confirm no
synthetic `{...}` names survive into `fq_path`. Minor related note: the
over-approximation `implementor_count` (mermaid.rs:187-188, via
`count_virtual_targets`) counts virtual edges from kept nodes even when the edge
was not actually drawn, so it can exceed the number of dashed edges in the
figure — an advisory number, harmless, but inconsistent with the drawn output.

Affects: callscope-core C8 renderer (P6), acceptance harness (P11).

---
Reconciliation 260726-2316: still OPEN. P11's C8 acceptance check strengthened the claim (asserts safe generated ids, no literal `\n`, classDefs present) but is a structural assertion, not a render against a real Mermaid v11 parser — the render-based verification this issue asks for is still absent. Low severity; no synthetic `{...}` names were observed reaching fq_paths in the fixture index. Does not block closure.
