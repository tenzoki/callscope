# Analyse: callscope vs. rust-query, zwei Lösungen desselben Tenders

**Datum:** 2026-07-26 23:39
**Typ:** Comparative
**Status:** Complete
**Angefragt von:** Nutzer (kai)

## Frage

Zwei unabhängige Implementierungen beantworten dieselbe Ausschreibung (`problem.md`, der callscope-Tender: ein Claude-Code-Plugin für compiler-gestützte Change-Impact-Antworten in Rust-Workspaces). Welche Lösung ist entlang der acht Achsen (Analysetechnik, Architektur, C1–C8, Q1–Q6, die vier Compiler-Lücken, Ehrlichkeit über Approximation, Test/Abnahme, Reife) wo stärker, und für welche Situation ist welche vorzuziehen?

- **Lösung A — callscope**: `/Users/kai/Dropbox/qboot/projects.4fun/260726-rust-callscope`
- **Lösung B — rust-query**: `/Users/kai/Dropbox/qboot/projects.4fun/260725-WG/repo/rust-query`

## Umfang und Methode

Ich habe beide Codebasen im Quelltext gelesen, nicht nur die Dokumentation: bei A die Crates `callscope-core` (Schema, Query, Envelope, Fingerprint, Mermaid), `callscope-index` (rustc-Treiber und Extraktion) und `callscope-mcp` (Server, Abnahme-Test) sowie die Fixture; bei B die Crates `rq-extractor` (Orchestrator, Extraktion), `rq-graph` (Facts, Loader, Query) und `rq-mcp` (Server) sowie `design.md`, die Fixture und die Tests.

**Kalibrierung der Aussagen.** Ich unterscheide streng: *geprüft* heißt, ich habe die Datei gelesen oder ein Kommando ausgeführt und zitiere die Fundstelle. Konkret ausgeführt habe ich nur eine Sache: ich habe den fertig gebauten Index von A (`fixtures/workspace/.callscope/index.bin`) mit einem eigenen Skript geparst und den Inhalt inspiziert. Beide Test-Suites habe ich **nicht** kompiliert oder laufen lassen (sie brauchen den gepinnten nightly-Compiler mit `rustc-dev`). Aussagen über bestandene Tests sind daher als `inference:` aus dem gelesenen Code und den in den Tests kodierten Ground-Truth-Assertions markiert.

## Vergleichstabelle

| Achse | Lösung A — callscope | Lösung B — rust-query | Vorteil |
|---|---|---|---|
| **1 Analysetechnik** | `rustc_public`/stable_mir + interner `TyCtxt`; monomorphisierte MIR via `Instance::resolve`; Kanten: Static, Virtual | `rustc_public` + `rustc_hir`/THIR; Zwei-Pass (polymorph + Monomorphisierungs-Worklist); Kanten: Static, Dynamic, FnPtr, Drop | B breiter (mehr Kantentypen, THIR) |
| **2 Architektur** | 3 Crates, `RUSTC_WRAPPER` + manuelle Member-Filter, Kaltneubau je Lauf, ein serialisierter `index.bin` + Manifest | 3 Crates, `RUSTC_WORKSPACE_WRAPPER`, inkrementelle Facts je Compile-Unit, kein Merge-Artefakt | gemischt (A: ein Artefakt; B: inkrementell) |
| **3 C1–C8** | 8 Tools, exakt C1–C8 | 11 Tools (C2 als 2 Tools, plus `index`/`status`); alle C1–C8 vorhanden | gleichwertig erfüllt |
| **4 Q1–Q6** | alle sechs; Q6 per Fingerprint auf jeder Anfrage | Q1–Q5 solide; **Q6 nur schwach** (nur "wann indexiert", keine Divergenzerkennung) | A bei Q6; B bei Q5-Kosten |
| **5 Vier Lücken** | Generics, dyn (inkl. generischer + crate-fremder Implementor), Closure/async, Makro-Tests inkl. `#[tokio::test]` | Generics, dyn, Closure/async, Makro-Tests; zusätzlich FnPtr; unsafe-Blöcke via THIR präziser | gemischt (A dyn-Breite; B FnPtr+unsafe) |
| **6 Ehrlichkeit** | strukturierter `over_approximated`-Grund + `implementor_count`, aber Trait-Label generisch `<dyn dispatch>` | `dynamic_edges_crossed`-Zähler; benannter Trait-Method-Hub sichtbar in Pfaden/Graph | gemischt |
| **7 Test/Abnahme** | dedizierte Abnahme C1–C8/Q1–Q6, 8 affected-tests, crate-fremder Implementor, Byte-Determinismus | E2E + Extraktions-Test; beweist Generic-Test + crate-fremden Integrationstest; kein dyn-only-Test, kein crate-fremder Implementor | A rigoroser |
| **8 Reife/Umfang** | ~5.450 Rust-Zeilen; README + SKILL sehr ausführlich; gebauter Index liegt vor | ~4.560 Rust-Zeilen; `design.md` exzellent; unter Git | gemischt |

Die Prosa-Begründung folgt achsenweise, jeweils These zuerst.

## Achse 1 — Analysetechnik und Toolchain

**These: Beide gewinnen die Ground Truth aus derselben Quelle (monomorphisierte MIR über `rustc_public`); B fährt die Extraktion mit mehr Compiler-Maschinerie und deckt dadurch mehr Ausführungsrealität ab, A ist dafür schlanker und im Kantenmodell fokussierter.**

Beide linken den Compiler ausschließlich in genau einer Crate und lesen die monomorphisierte MIR über `rustc_public` (den SemVer-verfolgten `stable_mir`-Nachfolger). A pinnt `nightly-2026-07-26` (`rust-toolchain.toml:13`), B `nightly-2026-07-23` (`rust-toolchain.toml:4`); beide ziehen `rustc-dev`, `llvm-tools-preview`, `rust-src`. Die Anbindungs-Fragilität ist damit strukturell gleich: ein nightly-Bump kann beide brechen, und beide dokumentieren das offen.

Der Kern-Mechanismus ist bei beiden `Instance::resolve` zur Monomorphisierung (A: `crates/callscope-index/src/graph_build.rs:405`; B: `crates/rq-extractor/src/extract/mod.rs:170-190`). Der Unterschied liegt in der Tiefe:

- **A** führt einen Lauf: es sammelt lokale Nicht-generische Funktionen als Wurzeln, resolved jeden Call-Terminator und läuft die Worklist ab (`graph_build.rs:172-197`). Sein Kantenmodell hat genau zwei Arten: `Static` und `Virtual` (`callscope-core/src/schema.rs:100-107`).
- **B** führt zwei Pässe: Pass A läuft die *polymorphen* Körper der Definitionen, die die Worklist nicht abdecken kann (generische Funktionen, Closures), Pass B ist eine Monomorphisierungs-Worklist im Kani-Stil, die präzise resolved und in Member-Crate-Instanzen absteigt (`extract/mod.rs:152-190`). B nutzt zusätzlich `rustc_hir`/THIR (den internen `TyCtxt`) für die Unsafe-Block-Erkennung (`extract/mod.rs:131`). Sein Kantenmodell hat vier Arten: `Static`, `Dynamic`, `FnPtr`, `Drop` (`rq-graph/src/facts.rs:72-86`).

**Fazit Achse 1.** B ist analysetechnisch der breitere Entwurf: es modelliert indirekte Aufrufe über Funktionszeiger (`FnPtr`) und Destruktor-Aufrufe (`Drop`), die A gar nicht erfasst. A ist bewusst schlanker und nutzt den internen `TyCtxt` nur für zwei Charakteristika (`test`, `public`, siehe `graph_build.rs:110-113`), während B ihn auch für die Unsafe-Analyse heranzieht. Für reine Change-Impact-Fragen über den direkten Aufrufgraphen sind beide gleich fundiert; sobald indirekte Dispatch-Wege (Callbacks über `fn`-Zeiger) im Spiel sind, sieht B mehr.

## Achse 2 — Architektur

**These: Beide trennen compiler-gelinkt von stable sauber; A liefert ein einziges deterministisches Index-Artefakt, B verzichtet auf ein Merge-Artefakt zugunsten inkrementeller Per-Unit-Facts. Das ist der zentrale Architektur-Trade-off zwischen den Lösungen und schlägt direkt auf Q5 und Q6 durch (Achse 4).**

Die Crate-Aufteilung ist bei beiden dreiteilig und in der Compiler-Isolation identisch im Prinzip:

- **A:** `callscope-core` (stable, Schema/Query/Envelope), `callscope-index` (nightly, einzige Compiler-Crate), `callscope-mcp` (stable, Server). Bestätigt in `README.md:33-38`.
- **B:** `rq-graph` (stable, Facts/Graph/Query), `rq-extractor` (nightly, einzige Compiler-Crate), `rq-mcp` (stable, Server). Bestätigt in `design.md:44-47`.

Die Indexierungsstrategie unterscheidet sie deutlich:

- **A** setzt `RUSTC_WRAPPER` und filtert Workspace-Member selbst über `CARGO_PRIMARY_PACKAGE` (`driver.rs:42-57`). Es **wischt bei jedem Lauf das Scratch-Verzeichnis** und erzwingt einen Kaltneubau aller Member (`main.rs:86-90`). Begründung im Code: cargos Inkremental-Cache würde einen Member aus einem früheren Build bedienen, der Wrapper liefe dann nicht, und der betroffene Member verschwände still aus dem Graphen. A merged danach die Fragmente zu einem einzigen `index.bin` und sortiert Symbole und Kanten für ein **byte-deterministisches** Artefakt (`main.rs:296-308`).
- **B** setzt `RUSTC_WORKSPACE_WRAPPER` (cargo ruft den Wrapper nur für Member auf, Probes und Build-Skripte laufen automatisch am realen rustc vorbei) und nutzt ein privates `CARGO_TARGET_DIR`, sodass der Nutzer-Build unberührt bleibt und cargos Inkremental-Skip **inkrementelle Facts geschenkt** liefert (`design.md:59-73`). Dasselbe Problem, das A per Kaltneubau löst (verschwindende gecachte Member), löst B eleganter: es leitet aus cargos `compiler-artifact`-Meldungen den erwarteten Facts-Datei-Satz ab und erzwingt genau einen vollen Rebuild, wenn erwartete Facts fehlen; Reste werden per GC entfernt (`design.md:69-73`, geprüft gegen die GC-Assertion in `tests/extract_fixture.rs:340-360`).

`RUSTC_WORKSPACE_WRAPPER` ist der idiomatischere Hebel; A muss die Member-Auswahl von Hand nachbauen. B lädt die nötige Compiler-dylib über einen im Binary eingebetteten rpath (`design.md:154-156`), A setzt zur Orchestrierungszeit `DYLD_FALLBACK_LIBRARY_PATH`/`LD_LIBRARY_PATH` (`main.rs:106-108`); Bs Weg ist für das ausgelieferte Binary sauberer.

**Fazit Achse 2.** Bs Indexierung ist die reifere Ingenieursleistung: sie löst das Cache-Kohärenz-Problem, ohne bei jeder Änderung alles neu zu übersetzen, und trifft damit genau die Q5-Anforderung "innerhalb einer normalen Editiersitzung neu indexieren". A kauft sich mit dem Kaltneubau ein byte-deterministisches Artefakt und eine einfache Kohärenz-Garantie, zahlt dafür aber auf großen Workspaces mit voller Neu-Übersetzung je Lauf. A hat dafür ein persistiertes, versioniertes Schema-Artefakt (`index.bin` + `manifest.json`) mit expliziter Kollisions-Erkennung beim Merge (`main.rs:183-241`), was Bs reiner In-Memory-Snapshot nicht kennt.

## Achse 3 — Abdeckung C1–C8

**These: Beide decken alle acht Fähigkeiten ab und exponieren sie als MCP-Tools. A bildet C1–C8 auf exakt acht Tools ab; B hat elf Tools (C2 in zwei aufgeteilt, plus operative Tools) und übertrifft A bei der Reichhaltigkeit einzelner Antworten, während A bei der Vollständigkeit der Pfad-Aufzählung (C4) und der inline-Lesbarkeit des Graphen (C8) vorn liegt.**

Wichtig zur Fairness: Bs `design.md:149` listet die Tools unvollständig auf und lässt `unsafe_callees` (C6) und `call_graph` (C8) weg. Der tatsächliche Server exponiert beide (`rq-mcp/src/server.rs:329` und `:349`, geprüft). Es fehlt also nichts; nur die Doku-Prosa ist veraltet.

| Fähigkeit | A (Tool) | B (Tool) | Anmerkung |
|---|---|---|---|
| C1 Symbol auflösen | `resolve_symbol` | `resolve` | beide geben Kandidatenmenge, kein stilles Raten |
| C2 direkte Aufrufer/Aufgerufene | `direct_calls` | `callers` + `callees` | B teilt in zwei Tools |
| C3 transitive Erreichbarkeit | `reachability` | `reachable` | B liefert zusätzlich Per-Crate-Zählung |
| C4 Pfade zwischen zwei Funktionen | `call_paths` | `paths` | A: alle einfachen Pfade; B: nur kürzeste |
| C5 betroffene Tests | `affected_tests` | `affected_tests` | B liefert je Test eine Beispiel-Kette |
| C6 erreichbares unsafe | `reachable_unsafe` | `unsafe_callees` | unterschiedliche Semantik (siehe Achse 5) |
| C7 kombinierter Impact | `impact` | `impact` | beide bündeln Aufrufer + Tests |
| C8 Graph-Visualisierung | `neighborhood_graph` (Mermaid) | `call_graph` (Graphviz DOT/JSON) | siehe unten |

Zwei Fähigkeiten lohnen die genauere Betrachtung:

- **C4.** A zählt per DFS **alle einfachen Pfade** bis zur Tiefengrenze auf und behandelt dabei Truncation, Depth-Cut und die Weitergabe von Virtual-Flags auch für verworfene Pfade sorgfältig (`query.rs:368-478`). B zählt nur **kürzeste Pfade** über den Kürzeste-Pfade-DAG auf (`query.rs:334-362`). Beide erfüllen C4; A ist vollständiger, B liefert die relevantesten Repräsentanten.
- **C8.** A rendert einen Mermaid-`flowchart`, der in der Claude-Oberfläche inline lesbar ist und strikt v11-sicher konstruiert wird (`mermaid.rs:28-38`). B rendert Graphviz-DOT mit Crate-Clustern, Modul-Sub-Clustern, Tooltips und Kanten-Stilen (`query.rs:810-895`), zusätzlich als JSON. Bs DOT ist für große, ernsthafte Visualisierungen reicher, braucht aber `dot` zum Rendern (das Skill nennt die `.dot`-Datei ehrlich als eigenständiges Ergebnis, `SKILL.md:56-58`). Für den Agenten-Kontext des Plugins ist As Inline-Mermaid unmittelbarer; für Diagramm-Tiefe ist Bs DOT überlegen.

Bs Reichhaltigkeit bei C3/C5/C7 ist ein echter Nutzungsvorteil: `reachable` und `impact` liefern Per-Crate-Zählungen (`query.rs:308-331`, `:407-430`), und `affected_tests` gibt zu jedem Test eine Beispiel-Aufrufkette (`query.rs:433-451`). Bei A müsste der Agent für die Kette einen separaten `call_paths`-Aufruf nachschieben.

**Fazit Achse 3.** Voll erfüllt bei beiden. B ist bei den zusammengesetzten Antworten informativer (Beispiel-Ketten, Per-Crate-Zählung), A ist bei C4 vollständiger und bei C8 im Agenten-Kontext direkter lesbar.

## Achse 4 — Qualitätsanforderungen Q1–Q6

**These: A erfüllt alle sechs, mit dem klaren Alleinstellungsmerkmal bei Q6 (erkennbare Staleness per Quell-Fingerprint auf jeder Anfrage). B erfüllt Q1–Q5 solide und ist bei Q5-Kosten überlegen, hat aber bei Q6 eine echte Lücke: es kann nicht erkennen, dass der Index dem aktuellen Quellstand hinterherhinkt, es meldet nur, wann zuletzt indexiert wurde.**

- **Q1 (Ground Truth).** Beide leiten aus dem Compiler ab, nicht aus Textabgleich; alle vier Lücken adressiert (Achse 5).
- **Q2 (sichtbare Unsicherheit).** Beide machen Über-Approximation sichtbar. A trägt einen strukturierten `over_approximated: Reason::DynDispatch { implementor_count }` auf einem einheitlichen Envelope, den *jedes* Tool zurückgibt (`envelope.rs:38-83`). B trägt einen Zähler `dynamic_edges_crossed` auf `reachable`, `affected_tests`, `unsafe_callees` und (über affected_tests) `impact` (`query.rs:126-201`). As Signal ist semantisch expliziter; Bs ist ein Zähler, den das Skill erklären muss (`SKILL.md:78-80`).
- **Q3 (kein stilles Raten).** Beide geben bei mehrdeutigen Namen die Kandidatenmenge zurück (A: `query.rs:264-281` und der Resolve-Then-Pfad in `tools.rs:160-171`; B: `query.rs:217-245`).
- **Q4 (LLM-taugliche Ausgabe).** Beide kappen und melden `total` plus `truncated`. Gleichwertig.
- **Q5 (wiederholbares Indexieren).** Beide sind wiederholbar. B ist bei den Kosten überlegen, weil es cargos Inkremental-Übersetzung nutzt (Achse 2), was der Q5-Formulierung "innerhalb einer normalen Editiersitzung" auf großen Workspaces besser entspricht. A erzwingt einen Kaltneubau, garantiert dafür Byte-Determinismus (verifiziert im Abnahme-Test, `acceptance.rs:565-577`).
- **Q6 (erkennbare Staleness).** Hier liegt der schärfste Unterschied. **A** schreibt ein `manifest.json` mit FNV-1a-Content-Hashes jeder `.rs`-Datei plus `Cargo.lock` (`fingerprint.rs:73-83`, im echten Index gesehen: die Manifest-Datei listet die drei Quelldateien mit Hashes) und prüft auf **jeder** Tool-Anfrage, welche Dateien divergiert sind (`fingerprint.rs:102-126`); das Ergebnis wird als `stale` mit `diverged_files` im Envelope zurückgegeben. **B** hat keinen Quell-Fingerprint. Sein `status`-Tool meldet `indexed_seconds_ago` und eine Notiz "re-index after code changes" (`server.rs:238-263`), und `design.md:194` sagt ausdrücklich "Facts staleness is by explicit re-index; no file watching." B kann also **nicht erkennen**, dass der Snapshot veraltet ist; es weiß nur, wann er gebaut wurde.

**Fazit Achse 4.** A ist bei Q6 klar überlegen und erfüllt die Anforderung im wörtlichen Sinn ("es muss erkennbar sein, dass Antworten aus einem Index stammen, der dem Quellstand vorausgeht"). B ist bei Q5-Kosten überlegen. Bs Gegenargument, inkrementelles Neu-Indexieren sei so billig, dass man einfach immer neu indexiert, ist praktisch tragfähig, entkräftet aber die formale Q6-Anforderung nicht.

## Achse 5 — Die vier Compiler-Lücken

**These: Beide schließen alle vier Lücken für den Kernfall. A behandelt die dyn-Über-Approximation adversarialer (generischer Implementor und crate-fremder Implementor sind im Fixture erzwungen und im gebauten Index sichtbar). B ist bei zwei Aspekten präziser: es modelliert Funktionszeiger-Indirektion (FnPtr) und erkennt user-geschriebene `unsafe {}`-Blöcke über THIR, wo As Heuristik still unter-approximieren kann.**

**Lücke 1 (Generics/Monomorphisierung).** Beide resolven `run_generic::<Simple>` präzise auf `<Simple as Tokenizer>::tokenize` als *statische* Kante. Bei A im gebauten Index verifiziert: das Symbol `parser::run_generic::<parser::Simple>` existiert und ruft `<parser::Simple as parser::Tokenizer>::tokenize` statisch. Bei B im Extraktions-Test kodiert (`extract_fixture.rs:220-225`: `run_generic -> <Simple as Tokenizer>::tokenize`, `Static`). `inference:` beide korrekt.

**Lücke 2 (dyn-Dispatch).** Beide über-approximieren zu allen Implementoren und flaggen das. Der Mechanismus unterscheidet sich: A verschiebt die Implementor-Aufzählung bewusst in den workspace-weiten Merge, sammelt je Crate `DynCall`s und `ImplMethodFact`s und joined sie global (`graph_build.rs:55-104`, Merge in `main.rs:261-283`). B routet jede dyn-Kante durch einen benannten Trait-Method-Hub-Knoten, und der Loader fächert `implements`-Links workspace-weit auf (`graph.rs:88-108`).

Der entscheidende Unterschied ist die **Fixture-Strenge**: As Fixture erzwingt zwei harte Fälle, die eine naive Über-Approximation still verfehlen würde, und ich habe im gebauten Index verifiziert, dass beide getroffen werden. `parser::run_dyn` fächert auf alle vier Implementoren auf, darunter `<parser::Wrapper<T> as parser::Tokenizer>::tokenize` (ein *generischer* Implementor) und `<ext_tokenizer::Shouty as parser::Tokenizer>::tokenize` (ein Implementor in einem *anderen* Member-Crate). Bs Fixture hat nur `Simple` und `Fancy`, beide im selben Crate wie Trait und `run_dyn`. Bs Loader würde crate-fremde und generische Implementoren mechanisch mitnehmen (der `implements`-Join ist workspace-weit), aber das Fixture *beweist* diese beiden schweren Fälle nicht.

**Lücke 3 (Closures/async).** Beide falten Closure- und Coroutine-Körper in die umschließende Funktion. A tut das über die *Typen* der Locals (es sucht Closure-Typen rekursiv in allen Local-Typen, was auch nicht-fangende Closures fängt, die als ZST verschwinden — dokumentiert als bewusster Bugfix in `graph_build.rs:338-362`). B tut es über das Hochlaufen der internen `DefId`-Elternkette, was zudem korrekt eine Closure im generierten Test-Deskriptor-Const verwirft statt sie fälschlich an eine gleichnamige Funktion zu kleben (`extract/mod.rs:225-233`). Beide Ansätze sind durchdacht. As Typ-basierte Erkennung ist gegen den ZST-Fall robuster dokumentiert; Bs DefId-Ansatz ist gegen die Harness-Namenskollision robuster.

**Lücke 4 (Makro-Tests + unsafe).** Bei der Test-Erkennung sind beide gleich gründlich und beide haben denselben Fallstrick erkannt: das `#[rustc_test_marker]`-Attribut ist über die stabile API nicht mehr sichtbar. A identifiziert den Harness-Const über seinen ADT-Typ `test::TestDescAndFn` und den geteilten fq_path (`graph_build.rs:511-539`); B scannt die HIR nach denselben Consts (`design.md:97-102`). A exerziert im Fixture zusätzlich `#[tokio::test]` (im gebauten Index als `async_reaches_target`, korrekt `test`-getaggt); B exerziert nur `#[test]` und verlässt sich auf das Argument, dass tokio-Wrapper zu `#[test]` expandieren.

Bei **unsafe** trennen sich die Wege am schärfsten. B unterscheidet drei Dinge: `is_unsafe` (die Signatur ist `unsafe fn`), `uses_unsafe` (der Körper enthält einen user-geschriebenen `unsafe {}`-Block, via THIR-Visitor, da unsafe-Blöcke in MIR gelöscht sind) und `unsafe_callee` je Kante (`facts.rs:42-69`, `extract/mod.rs:131`). As `uses_unsafe` ist eine *Heuristik*: es ist wahr, wenn die Funktion `unsafe fn` ist oder eine Funktion mit unsafe-Signatur aufruft (`graph_build.rs:564-577`). Das verfehlt einen sicheren `fn` mit einem primitiven unsafe-Block, der keine unsafe-Funktion aufruft (etwa `unsafe { *ptr }`, ein reiner Raw-Pointer-Deref). Genau so einen Fall hat Bs Fixture (`ffi.rs:12-14`, `read_first`), und Bs THIR-Visitor fängt ihn. `inference:` As `uses_unsafe` würde bei einem sicheren Wrapper mit reinem Raw-Deref-Block still `false` melden, obwohl es unsafe-Code gibt. As Doku sagt "Contains an unsafe block or is an unsafe fn" (`schema.rs:80-82`), die Implementierung liefert das aber nur näherungsweise; im As-Fixture fällt das nicht auf, weil `ensure_round_trip` `std::slice::from_raw_parts` aufruft (eine unsafe-Signatur, die die Heuristik trifft).

**Kritischer Ehrlichkeitspunkt (Q2).** Der Tender nennt eine `dyn`-Antwort, der Implementoren fehlen, ausdrücklich als Defekt. Keine der beiden Lösungen begeht diesen Defekt bei dyn: beide fächern vollständig auf und flaggen. Die eine *stille Unter-Approximation*, die ich gefunden habe, ist As `uses_unsafe`-Heuristik bei C6 (primitiver unsafe-Block ohne unsafe-Aufruf), nicht bei dyn. Das ist ein realer, wenn auch enger Präzisionsmangel bei A, bei dem B korrekter ist.

**Fazit Achse 5.** Alle vier Lücken sind bei beiden für den Kernfall geschlossen. A ist bei der dyn-Über-Approximation adversarialer bewiesen (generischer + crate-fremder Implementor) und exerziert `#[tokio::test]`. B ist bei zwei Aspekten präziser: FnPtr-Indirektion (die A nicht modelliert) und THIR-genaue unsafe-Block-Erkennung (wo A still unter-approximieren kann).

## Achse 6 — Umgang mit Über-/Unter-Approximation und Ehrlichkeit

**These: Beide machen Unsicherheit, Truncation und Grenzen sichtbar. As Signal ist strukturell prominenter (ein benannter Über-Approximations-Grund auf jedem Envelope), benennt aber den auslösenden Trait nur generisch. Bs Signal ist ein Zähler, dafür ist der reale Trait-Method-Hub in Pfaden und Graph namentlich sichtbar. Kein Ansatz ist perfekt; sie tauschen Prominenz gegen Benennung.**

- **A:** Der Envelope ist ein einheitlicher Typ, den jedes Tool zurückgibt (`envelope.rs:59-83`); `over_approximated`, `stale`, `truncated`/`total`, `boundary_applies` sind Felder darauf. Das ist die stärkere strukturelle Garantie: die Unsicherheit kann nicht je Tool abweichend geformt sein. Aber: der `Reason::DynDispatch.trait_path` ist der generische Marker `<dyn dispatch>`, nicht der konkrete Trait (`query.rs:40-46` und `mermaid.rs:46-49`). Das SKILL zeigt in seinem Beispiel `trait_path: "parser::Tokenizer"` (`SKILL.md:108`), was die Implementierung so nicht liefert. Das ist eine Doku-gegen-Code-Abweichung: der `implementor_count` ist korrekt, der Trait-Name ist es nicht. Kein Defekt an der Über-Approximation selbst, aber eine Ungenauigkeit im gemeldeten Grund.
- **B:** Der Über-Approximations-Indikator ist der Zähler `dynamic_edges_crossed`. Er ist weniger selbsterklärend als ein benannter Grund, aber Bs Graph macht den auslösenden Trait *namentlich* sichtbar: dyn-Kanten laufen durch einen Knoten wie `lib_a::engine::Tokenizer::tokenize` (`is_trait_method`-geflaggt), der in `paths`- und `call_graph`-Ausgaben auftaucht (verifiziert in `extract_fixture.rs:206-211` und der DOT-Ausgabe in `mcp_e2e.rs:142-162`). Ein Konsument sieht also, *welcher* Trait dispatcht.

Beide ziehen die Workspace-Grenze ehrlich (dritt-Crate-Aufrufe werden nicht durchlaufen) und flaggen sie: A per `boundary_applies` (`query.rs:180-196`), B per `external`-Knoten und die Skill-Erklärung (`SKILL.md:83-85`). Beide melden Truncation mit wahren Gesamtzahlen.

**Fazit Achse 6.** Ehrlichkeit ist bei beiden ernst genommen und im Kern erfüllt. As einheitlicher Envelope ist die robustere strukturelle Zusage; sein generisches Trait-Label und die Doku-Abweichung sind ein kleiner Malus. Bs Zähler ist knapper, dafür ist der reale Trait im Graphen benannt. Unentschieden mit unterschiedlichen Stärken.

## Achse 7 — Test und Abnahme

**These: Beide haben ernsthafte End-to-End-Tests über den echten Compiler. As Abnahme ist die rigorosere: ein dedizierter Harness prüft C1–C8 und Q1–Q6 einzeln, die Antwortmenge ist adversarialer (acht betroffene Tests, dyn-only-Tests, crate-fremder Implementor, Byte-Determinismus). Bs Tests beweisen den Kernfall (Generic-Dispatch-Test plus crate-fremder Integrationstest) sauber, sind aber weniger adversarial und prüfen keinen Determinismus.**

Das Leit-Beispiel des Tenders verlangt zwei Dinge in der affected-tests-Antwort für `normalize_token`: einen Test, der die Funktion *nur über generischen Trait-Dispatch* erreicht und sie nie beim Namen nennt, und einen Test aus dem Integrationstest-Target eines *anderen* Crates.

- **A** beweist beides und mehr. Der Abnahme-Test (`crates/callscope-mcp/tests/acceptance.rs`) baut den echten Index zweimal, lädt ihn über den Server-Pfad und prüft je eine Zeile für C1–C8, Q1–Q6, den dyn-Fall und die Grenze. Die affected-tests-Antwort enthält acht Tests (`acceptance.rs:69-78`), darunter `reaches_via_generic_dispatch` (in-crate, generischer Dispatch) und `integration_reaches_via_generic` (separates Crate, generischer Dispatch, nennt die Funktion nie). Ich habe die zugrunde liegende Ground Truth im gebauten Index verifiziert: alle acht Test-Symbole sind korrekt `test`-getaggt, und `run_dyn` fächert auf alle vier Implementoren auf. `inference:` der Abnahme-Test besteht (ich habe ihn nicht ausgeführt, aber der Index, aus dem er liest, liegt korrekt gebaut vor).
- **B** beweist den Kernfall im E2E-Test (`crates/rq-mcp/tests/mcp_e2e.rs:99-133`): die impact-Antwort für `normalize_token` enthält `normalizes_directly`, `generic_reaches_normalize` (in-crate, generischer Dispatch) und `integration::parses_via_lib` (separates Crate) und schließt `unrelated` aus. Der Extraktions-Test (`tests/extract_fixture.rs`) prüft jede Kanten-Art einzeln, inklusive Closure-Attribution, async-Kette, FnPtr-Reifikation und GC-Idempotenz. Das ist gründlich. Aber: der crate-fremde Integrationstest erreicht `normalize_token` über einen *statischen* Closure-Aufruf durch `parse`, nicht über generischen oder dyn-Dispatch; und es gibt in Bs Fixture *keinen* Test, der die Funktion nur über dyn-Dispatch erreicht (`run_dyn` wird allein aus `main` aufgerufen, nicht aus einem Test). Der Buchstabe von §6 ist erfüllt (ein Generic-Test *und* ein crate-fremder Test), die adversariale Kombination aus As Fixture (crate-fremder Test *über* Dispatch, crate-fremder Implementor) fehlt.

`inference:` Bs Tests bestehen ebenfalls (ich habe sie nicht ausgeführt; die Assertions kodieren die erwartete Ground Truth und sind konsistent mit dem gelesenen Extraktor).

Zum Determinismus: A prüft Byte-Identität zweier Index-Läufe explizit (`acceptance.rs:565-577`). B prüft Idempotenz des Facts-Satzes und den GC (`extract_fixture.rs:340-360`), aber nicht die Byte-Gleichheit der Facts.

**Fazit Achse 7.** Beide sind ernsthaft getestet. A ist rigoroser und adversarialer, mit einem dedizierten C1–C8/Q1–Q6-Harness, härteren Fixture-Fällen und einem Determinismus-Nachweis. B ist gründlich auf Kanten-Ebene und beweist den geforderten Kernfall, ist aber in der affected-tests-Konstellation weniger scharf.

## Achse 8 — Reife, Robustheit, Umfang, offene Punkte

**These: Beide sind gut dokumentiert und in vergleichbarer Reife. B ist etwas kompakter im Code und hat die exzellentere Design-Erklärung; A hat die ausführlichere Nutzer-Doku und ein vorliegendes, gebautes Index-Artefakt. Beide benennen ihre Grenzen ehrlich.**

Umfang (grob, inklusive Testcode): A rund 5.450 Rust-Zeilen über drei Crates, B rund 4.560 über drei Crates. Bs `rq-graph` ist mit ~2.170 Zeilen die schwerste Einzel-Crate (viel davon die DOT-Rendering-Logik), As `callscope-core` mit ~2.500 die schwerste.

Dokumentation: Bs `design.md` ist die stärkere Architektur-Erklärung (sie legt Zwei-Pass, Worklist, Shim-Behandlung, Facts-Schema und Loader präzise offen) und benennt die Grenzen sauber (`design.md:182-194`: nightly-Bindung, workspace-only, FnPtr-Approximation, identisch-gedruckte generische Impls können mergen, keine File-Watches). As `README.md` und `SKILL.md` sind die ausführlichere *Nutzer*-Doku, mit einem eigenen Abschnitt "Honest limitations" (`README.md:114-133`). A benennt seine Grenzen ebenso offen (dyn über-approximiert, Workspace-Grenze, Staleness). B steht unter Git (drei Commits), A ist nicht unter Versionskontrolle; A hat dafür einen fertig gebauten, korrekten Index im Fixture liegen.

Symbol-Identität: A verwendet einen FNV-1a-Hash des fq_path als `SymbolId` mit expliziter Kollisions-Erkennung im Merge (`schema.rs:19-54`, `main.rs:232-241`). B verwendet den gedruckten Definitionspfad direkt als Id und nennt als bekannte Grenze, dass identisch gedruckte generische Impls mergen können (`design.md:192`). Beide teilen dieselbe fundamentale Grenze (gleicher gedruckter Pfad = ein Knoten); A hat ein zusätzliches Sicherheitsnetz gegen Hash-Kollisionen unterschiedlicher Pfade.

**Fazit Achse 8.** Vergleichbare Reife. B ist im Code etwas dichter und in der Architektur-Doku brillanter; A in der Nutzer-Doku ausführlicher und mit lauffähigem Artefakt.

## Schluss-Fazit

**These: Beide Lösungen sind ernsthafte, compiler-fundierte Implementierungen, die den Tender im Kern erfüllen. Sie sind unterschiedlich stark, nicht unterschiedlich gut. A gewinnt bei Ehrlichkeits-Infrastruktur und Abnahme-Rigor (einheitlicher Envelope, Q6-Staleness, adversariales Fixture, Determinismus); B gewinnt bei analytischer Breite und Indexierungs-Ingenieurskunst (FnPtr/Drop-Kanten, THIR-unsafe, inkrementelle Facts, reichere Antworten).**

**Stärken A (callscope):**
- Einheitlicher Envelope über alle acht Tools, der Q2/Q4/Q6 und die Grenze als Felder trägt: die robusteste strukturelle Ehrlichkeits-Zusage.
- Q6 im Wortsinn erfüllt: Quell-Fingerprint-Prüfung auf jeder Anfrage mit benannten divergierten Dateien.
- Rigoroseste Abnahme: dediziertes C1–C8/Q1–Q6-Harness, adversariales Fixture (generischer und crate-fremder Implementor, dyn-only-Tests, `#[tokio::test]`), Byte-Determinismus.
- Deterministisches, versioniertes Index-Artefakt mit Kollisions-Erkennung.
- C4 vollständig (alle einfachen Pfade); C8 inline-lesbares Mermaid.

**Schwächen A:**
- `uses_unsafe` (C6) ist eine Heuristik, die einen sicheren `fn` mit primitivem unsafe-Block still unter-approximieren kann; Doku und Implementierung weichen ab.
- Über-Approximations-Grund benennt den Trait nur generisch (`<dyn dispatch>`); SKILL-Beispiel weicht ab.
- Kaltneubau je Index-Lauf: auf großen Workspaces teurer als nötig (Q5-Kosten).
- Kein FnPtr-/Drop-Modell: indirekte Aufrufe über Funktionszeiger werden nicht erfasst.

**Stärken B (rust-query):**
- Analytisch am breitesten: Zwei-Pass-Extraktion, FnPtr-Reifikation und Drop-Glue als eigene Kantenarten, THIR-genaue unsafe-Block-Erkennung.
- Reifste Indexierung: `RUSTC_WORKSPACE_WRAPPER` plus inkrementelle Per-Unit-Facts mit korrektem GC, ohne den Nutzer-Build zu berühren: trifft Q5 auf großen Workspaces besser.
- Reichere Antworten: Per-Crate-Zählungen, Beispiel-Aufrufketten je betroffenem Test, Trait-Method-Hub im Graphen benannt.
- Exzellente Architektur-Doku.

**Schwächen B:**
- Q6 nur schwach: keine Staleness-Erkennung, nur "wann indexiert" plus Notiz.
- Weniger adversariales Fixture: kein dyn-only-Test, kein crate-fremder Implementor, kein `#[tokio::test]`, kein Determinismus-Nachweis.
- Über-Approximations-Signal ist ein Zähler ohne strukturierten Grund; kein einheitlicher Envelope (Unsicherheitsfelder je Tool unterschiedlich geformt).
- C4 nur kürzeste Pfade; C8-DOT braucht externen Renderer.

**Wann welche vorzuziehen ist.**
- Wo **verlässliche Unsicherheits- und Staleness-Semantik** und ein **beweisbar korrektes, deterministisches Abnahmeverhalten** im Vordergrund stehen (der eigentliche Anspruch des Tenders an Ehrlichkeit), ist **A** vorzuziehen. As einheitlicher Envelope und die Per-Request-Staleness sind genau die Eigenschaften, die einen LLM-Konsumenten davor bewahren, einer stillen oder veralteten Antwort zu trauen.
- Wo **große, reale Workspaces** häufig neu indexiert werden müssen und **analytische Breite** (indirekte Aufrufe, Destruktoren, präzise unsafe-Blöcke, reiche Diagramme) zählt, ist **B** vorzuziehen. Bs inkrementelle Facts und die vier Kantenarten skalieren und sehen mehr Ausführungsrealität.

**Wichtigste konkrete Unterschiede in einem Satz je Paar:**
1. Staleness: A prüft Quell-Hashes auf jeder Anfrage; B meldet nur das Index-Alter.
2. Indexierung: A baut jeden Member je Lauf kalt neu; B nutzt cargos Inkremental-Cache mit persistenten Per-Unit-Facts.
3. Kantenmodell: A hat Static und Virtual; B hat zusätzlich FnPtr und Drop.
4. unsafe: A leitet `uses_unsafe` heuristisch aus unsafe-Aufrufen ab; B erkennt echte `unsafe {}`-Blöcke via THIR.
5. Unsicherheits-Ausgabe: A trägt einen strukturierten Grund auf einem einheitlichen Envelope; B trägt einen `dynamic_edges_crossed`-Zähler je Query.

## Gefilterte Issues

Keine. Dies ist eine vergleichende Analyse, keine Mängelaufnahme an einem einzelnen Projekt. Sollte A weiterentwickelt werden, wären zwei Punkte einen Issue wert: die `uses_unsafe`-Heuristik in `graph_build.rs:564-577` (stille C6-Unter-Approximation) und die Trait-Label-Abweichung zwischen `query.rs:46`/`SKILL.md:108`.

## Quellen

Geprüft (gelesen), Lösung A: `problem.md`; `rust-toolchain.toml`; `README.md`; `.mcp.json`; `.claude-plugin/plugin.json`; `skills/callscope/SKILL.md`; `crates/callscope-index/src/{driver.rs,graph_build.rs,main.rs}`; `crates/callscope-core/src/{schema.rs,query.rs,envelope.rs,fingerprint.rs,mermaid.rs}`; `crates/callscope-mcp/src/tools.rs`; `crates/callscope-mcp/tests/acceptance.rs`; `fixtures/workspace/parser/src/lib.rs`; `fixtures/workspace/parser/tests/integration.rs`; `fixtures/workspace/ext_tokenizer/src/lib.rs`.

Geprüft (ausgeführt), Lösung A: Parse und Inspektion von `fixtures/workspace/.callscope/index.bin` (21 Symbole, 93 Kanten, 8 Test-Symbole, vier Virtual-Implementoren an `run_dyn`) und `fixtures/workspace/.callscope/manifest.json`.

Geprüft (gelesen), Lösung B: `repo/rust-query/design.md`; `rust-toolchain.toml`; `plugin/.claude-plugin/plugin.json`; `plugin/.mcp.json`; `plugin/skills/analyze/SKILL.md`; `crates/rq-extractor/src/extract/mod.rs`; `crates/rq-graph/src/{facts.rs,graph.rs,query.rs}`; `crates/rq-mcp/src/{server.rs,indexer.rs}`; `crates/rq-mcp/tests/mcp_e2e.rs`; `crates/rq-extractor/tests/extract_fixture.rs`; `fixtures/sample/crates/lib_a/src/{lib.rs,parser.rs,engine.rs,ffi.rs}`; `fixtures/sample/crates/app/src/main.rs`; `fixtures/sample/crates/app/tests/integration.rs`; `git -C repo/rust-query log`.

Nicht ausgeführt: die Test-Suites beider Projekte (nightly-Compiler mit `rustc-dev` erforderlich). Aussagen über bestandene Tests sind entsprechend als `inference:` markiert.

## Offene Punkte

- [ ] Falls belastbare Performance-Aussagen gewünscht sind (Index-Zeit auf einem großen realen Workspace, wo Bs Inkrementalität gegen As Kaltneubau antritt), müssten beide gegen dasselbe nicht-triviale Ziel-Workspace gebaut und gemessen werden. Das war nicht Teil dieser Analyse.
- [ ] Die Aussage, dass beide Abnahme-Suites bestehen, ist Inferenz aus dem Code; ein tatsächlicher Lauf beider Suites würde sie zu "geprüft" heben.
