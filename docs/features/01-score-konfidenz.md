# 01 — Score-Konfidenz nach Upstream

**Status:** umgesetzt auf `feature/score-confidence` (2026-10-30), noch nicht gemergt
**Upstream:** [`134d170`](https://github.com/jaredpalmer/kev/commit/134d170) (2026-09-25),
`kev/api.py`: `score_confidence`, `choice_confidence`, `_normalize`

## Problem

`rkev/src/readout.rs` (`score_confidence`) rechnet noch die alte Näherung:

```
1 - E|level - mode| / (L - 1)
```

Upstream folgt seit 2026-09-25 dem Referenz-Adapter von TypeSafe
(`system-one-adapter` 0.2.1, `_utils/confidence_metrics.py`):

```
p    = normalisiert auf Summe 1 (alles 0 -> gleichverteilt)
mode = erstes wahrscheinlichstes Level
D    = mittlere absolute Abweichung einer Gleichverteilung über L Levels
       um ihren Mittelwert (L-1)/2, also sum(|i - (L-1)/2|) / L
conf = max(0, 1 - E|level - mode| / D)          (L == 1 -> 1)
```

Damit ist eine Gleichverteilung 0 statt eines positiven Werts, und alles, was
breiter streut, wird bei 0 abgeschnitten. Jede `score`-Antwort von rkev trägt
also heute eine andere `confidence` als der Server. Beispiel aus Upstreams Test:
`[0.1, 0.3, 0.6]` ergibt dort 0.25, bei uns 0.75.

`choice_confidence` normalisiert upstream ebenfalls vorher. Für eine Softmax ist
das wirkungslos, aber der Port soll die Formel genauso haben, damit er auch
dann stimmt, wenn er einmal etwas anderes als eine Softmax bekommt.

## Umsetzung

1. `tests/upstream_unit.rs` nachziehen, Testnamen wie upstream in
   `tests/test_unit.py`:
   - `test_to_answers_shapes_and_formulas`: die Score-Konfidenz `0.25` für
     `[0.1, 0.3, 0.6]` (fehlt bei uns bisher ganz).
   - `test_confidence_edge_cases`: gleichverteilt → 0 für L = 2..10;
     `[0.5, 0.0, 0.5]` → 0 (abgeschnitten); alles auf einem Level → 1.
     Den Fall „Ende-zu-Ende gesplittet: irgendwo in 0..1“ in
     `the_confidence_edge_cases_are_the_python_ones` durch den exakten
     Upstream-Wert (0) ersetzen.
   - `test_score_confidence_matches_typesafe_docs` neu: die fünf Beispiele aus
     docs.typesafe.ai (`[0, .57, .43]` → 0.35, `[0, .14, .86, 0, 0]` → 0.89,
     `[0, 0, .48, .52]` → 0.52, `[0, .74, .26]` → 0.61, `[0, 0, 0, 1]` → 1.0),
     Toleranz 0.011 wie upstream.
   - Die Fälle „normalisiert vorher“ und „alles 0“ lassen sich über den
     Stub-Backend nicht erzeugen (er liefert Softmaxe). Entweder über eine
     Testschnittstelle an der Formel oder ausdrücklich als nicht portiert
     vermerken, mit Begründung.
2. `score_confidence` und `choice_confidence` in `readout.rs` auf die Upstream-
   Formeln umstellen; Doc-Kommentar mit Verweis auf `kev/api.py` und den
   TypeSafe-Adapter.
3. Doku: Die Konfidenz ist weiterhin ein Formmaß der Verteilung, keine
   Trefferquote (Invariante in `CLAUDE.md`). Die Formel für `score` steht im
   Root-`README.md` (*Vom Ticket zur Antwort*), in `docs/ablauf.md` und
   `rkev/README.md`. Alle Stellen suchen (`grep -rn "L - 1\|mode" docs README.md rkev/README.md`)
   und anpassen.
4. `tests/parity/`: Falls eine Aufnahme eine `score`-Frage enthält, die vor
   2026-09-25 aufgenommen wurde, ist ihre `confidence` veraltet. Prüfen, ob
   `examples/parity` die Konfidenz überhaupt vergleicht.

## Fertig, wenn

- die drei Upstream-Tests portiert sind und grün laufen,
- `scripts/test.sh` mit der ganzen Feature-Matrix grün ist,
- keine Doku mehr die alte Formel nennt.

## Ergebnis

- Formel, Clip und Normierung wie upstream (`rkev/src/readout.rs`).
- Portiert: die Score-Zeile aus `test_to_answers_shapes_and_formulas`, die
  exakten Nullen aus `test_confidence_edge_cases` und
  `test_score_confidence_matches_typesafe_docs` in `tests/upstream_unit.rs`.
  Die Fälle, die eine Softmax nicht erzeugen kann (nicht normiert, alles 0),
  stehen als Unit-Test in `readout.rs`. Jeder neue Test wurde gegen die alte
  Formel rot gesehen.
- Das README-Beispiel von Kev sagt jetzt 0.34 statt 0.78. Fixtures und
  `rkev/README.md` folgen ihm. Der gemessene Lauf im Root-`README.md` trägt
  einen Vermerk, weil er neu gemessen werden müsste.
- `examples/parity` vergleicht keine Konfidenz, und es gibt keine Aufnahmen.
  Hier war also nichts zu tun. Hätte parity sie verglichen, wäre diese
  Abweichung dort aufgefallen.
