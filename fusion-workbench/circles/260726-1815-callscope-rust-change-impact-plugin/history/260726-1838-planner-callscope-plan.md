# Planner session — callscope implementation plan

**Status:** Complete
**Date:** 2026-07-26 18:38
**Agent:** planner
**Circle:** 260726-1815-callscope-rust-change-impact-plugin

## What was done

Produced the implementation plan for the callscope Claude Code plugin from the Circle record (spec-equivalent) and `problem.md` (authoritative C1–C8, Q1–Q6, §6 acceptance, §7 boundary).

Research Gate: no internal codebase to survey (new project), so surveyed the external Rust static-analysis ecosystem. Web search confirmed the analysis technique. Findings:
- `rustc_public` (formerly `stable_mir`) is the Rust project's public MIR API for external tools, shipped in nightly `rustc-dev`.
- `rustc_monomorphize::collector` closes gaps 1–3 natively (generic instantiation, dyn over-approximation, closure/async body collection); HIR attributes + unsafety check close gap 4.
- Prior art: cargo-call-stack, nrc/callgraph.rs, Kani, MIRAI.
- rust-analyzer alone cannot close gap 1 (no monomorphization) — ruled out as standalone.

Design: one cargo workspace, three crates. `callscope-core` (stable) owns schema, fingerprint, query algorithms, and the one output envelope. `callscope-index` (nightly rustc driver) is the only compiler-linked component. `callscope-mcp` (stable) serves C1–C8. Plus a workflow skill and plugin packaging. Unifying move: a single `Envelope<T>` carries Q2/Q4/Q6/boundary reporting so the eight tools don't fragment into ad-hoc shapes.

11 dependency-ordered steps, all routed to coder. ontocoder has no v1 step (no hand-authored structured data; manifest is code-generated — a hand-authored schema would violate single-source-of-truth). Step 4 (indexing engine) flagged as needing user approval of the toolchain decision.

## Artifacts

- Plan: `circles/260726-1815-callscope-rust-change-impact-plugin/planning/260726-1838_o_callscope-implementation.md`
- Decision (open, needs user ratification): `circles/260726-1815-callscope-rust-change-impact-plugin/decisions/260726-1838_o_analysis-technique-and-toolchain.md`

## Notes

- SearXNG MCP was down; used built-in WebSearch instead.
- Voice profiles loaded: chat-voice-en.yaml, default-voice-en.yaml (both present, no fallback).
- Plan contains three Mermaid diagrams (component shape, indexing pipeline, and inline step ordering via the two flowcharts); a conceptrev pass can evaluate them at the plan gate.
