# Analyst-Session: callscope vs. rust-query Vergleich

**Datum:** 2026-07-26 23:39
**Agent:** analyst
**Status:** Complete

## Auftrag

Fairer, kalibrierter Vergleich zweier unabhängiger Lösungen desselben Tenders (`problem.md`, callscope Change-Impact-Plugin). Lösung A = callscope (dieses Projekt), Lösung B = rust-query (`260725-WG/repo/rust-query`). Vergleichsraster über acht Achsen. Read-only.

## Vorgehen

Beide Codebasen im Quelltext gelesen (nicht nur Docs). As gebauten `index.bin` mit eigenem Skript geparst und verifiziert. Beide Test-Suites nicht ausgeführt (nightly rustc-dev nötig); Aussagen dazu als `inference:` markiert.

## Kernbefunde

- Beide erfüllen den Tender im Kern; unterschiedlich stark, nicht unterschiedlich gut.
- A stärker: einheitlicher Envelope, Q6-Staleness per Fingerprint auf jeder Anfrage, rigoroseste Abnahme (C1–C8/Q1–Q6-Harness, adversariales Fixture mit generischem + crate-fremdem Implementor + `#[tokio::test]`), Byte-Determinismus, C4 vollständig, C8 inline-Mermaid.
- B stärker: analytische Breite (FnPtr/Drop-Kanten, Zwei-Pass-Worklist, THIR-genaue unsafe-Blöcke), inkrementelle Indexierung (Q5-Kosten), reichere Antworten (Per-Crate-Zählung, Beispiel-Ketten), Trait-Hub im Graphen benannt, exzellente `design.md`.
- Eine stille Unter-Approximation gefunden, bei A: `uses_unsafe` (C6) ist Heuristik, verfehlt sicheren fn mit primitivem unsafe-Block. Kein dyn-Defekt bei beiden.
- Kleinere As-Ungenauigkeit: Über-Approximations-Grund benennt Trait generisch (`<dyn dispatch>`), SKILL-Beispiel weicht ab.
- Bs Q6-Lücke: keine Staleness-Erkennung, nur Index-Alter.

## Ausgabe

Bericht: `fusion-workbench/shared/analyses/260726-2339-callscope-vs-rust-query-vergleich.md`

## Notiz

Deutsche Voice-Profile geladen (`default-voice-de.yaml`, `chat-voice-de.yaml`), da Bericht auf Deutsch. Keine Issues gefiled (vergleichende Analyse). Zwei potenzielle Issues für A notiert, falls Weiterentwicklung: `uses_unsafe`-Heuristik und Trait-Label-Abweichung.
