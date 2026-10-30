# 03 — Kontextlimit 65.536 Tokens

**Status:** offen, setzt 02 voraus
**Upstream:** [`9689666`](https://github.com/jaredpalmer/kev/commit/9689666) (2026-09-26),
`kev/model.py`: `SERVE_MAX_STATE = 65536`, `SERVE_MAX_BRANCH = SERVE_MAX_STATE + 8192`;
dazu für den Speicher [`71d4829`](https://github.com/jaredpalmer/kev/commit/71d4829)
(fp32-Attention mit linearem Speicher für lange Zeilen) und
[`1d77363`](https://github.com/jaredpalmer/kev/commit/1d77363) (MLX: State in
Stücken von 1.024 Tokens vorfüllen)

## Problem

`Limits::serving()` steht bei uns auf 8192 / 8192, upstream bei 65.536 für den
State und 73.728 für State plus einen Zweig. Ohne Feature 02 heißt das, dass ein
State zwischen 8k und 64k bei uns still gekürzt wird und beim Server nicht. Mit
Feature 02 lehnen wir ab, wo der Server antwortet.

## Warum das nicht nur eine Zahl ist

Upstream hat für 64k eigene Arbeit gebraucht, nämlich speichereffiziente
Attention-Kernel und Prefill in Stücken. rkev läuft auf candle, und dort bauen
die Pfade volle Masken und Score-Matrizen:

- Der gepackte Pass baut eine Maske `L x L`. Bei 65k sind das 4,3 Mrd. Einträge,
  in f32 17 GB, und dazu kommen die Scores pro Kopf.
- Auch der Prefix-Pfad rechnet die kausale Attention über den ganzen State in
  einem Stück.

Die Zahl einfach hochzusetzen, macht aus einem sauberen 413/422 also einen
Speicherabsturz. Das wäre schlechter als jetzt.

## Umsetzung

1. **Erst messen:** Wie lang darf ein State auf der CPU und auf Metal sein,
   bevor ein Pass mit kev-0.6b bzw. kev-0.8b nicht mehr passt? Das als
   `#[ignore]`d-Messung festhalten, wie die anderen in *Making it fast enough*.
2. **Chunked State-Prefill**, wie bei MLX upstream: Den State in Stücken von
   1.024 Tokens durch den Prefix-Pfad schicken. Attention liest den
   KV-Cache der vorigen Stücke, und die Rekurrenz trägt ihren Zustand weiter.
   Das ist exakt bis auf die Reihenfolge der Float-Additionen. Über einer
   Schwelle immer den Prefix-Pfad nehmen statt des gepackten Passes. Tests:
   gleiche Antworten wie ungeteilt auf beiden Fixture-Backbones und gegen die
   Transkription.
3. Erst danach `Limits::serving()` auf 65.536 / 73.728 setzen und die Docs von
   `Limits` nachziehen. Der Prefix-Cache (4 States) wird bei 64k-States
   gigabytegroß. Upstream begrenzt ihn mit `KEV_PREFIX_MAX_TOKENS = 65536`
   insgesamt, bei uns fehlt ein Token-Budget dafür.
4. **Entscheidung nötig:** Falls 2 nicht ins Budget passt, gibt es eine
   ehrliche Alternative. 8192 bleibt Standard und wird als bewusste Abweichung
   von `kev.serve` dokumentiert. Dank 02 ist sie dann laut (Ablehnung) statt
   still. 65.536 bekommt nur, wer es mit `with_limits` ausdrücklich verlangt.

## Was die Doku sagen muss

Für `kev-0.8b`, `kev-4b` und `kev-9b` ist laut den 1.0-Modellkarten ein Kontext
von **8.192 Tokens validiert**. Bei 16k liegen alle drei schon außerhalb der
Toleranz (untere Grenzen −8,5 / −3,4 / −3,7 Prozentpunkte auf CUAD). Trainiert
wurden sie auf States bis 7.552 Tokens. Dass ein langer State angenommen wird,
heißt also nicht, dass die Antwort gedeckt ist. Das gehört neben das Limit in
`rkev/README.md`, das Root-`README.md` und `docs/ablauf.md`.

## Fertig, wenn

- ein 64k-State auf der CPU entweder beantwortet oder sauber abgelehnt wird,
  nie mit einem Absturz,
- das gewählte Limit gemessen begründet und dokumentiert ist.
