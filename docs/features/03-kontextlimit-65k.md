# 03 — Kontextlimit 65.536 Tokens

**Status:** umgesetzt auf `feature/long-context` (2026-10-30) als „8.192 + Attention in Stücken“, noch nicht gemergt
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

## Gemessen (2026-10-30)

`what_a_long_state_costs_one_attention_layer` in `rkev/src/weights.rs`
(`#[ignore]`d, `cargo test --release --features candle --lib -- --ignored
--nocapture`): Spitzenspeicher **einer** Attention-Schicht beim Prefill eines
States, mit den Formen von kev-0.6b (16 Query-Köpfe, 8 KV-Köpfe, 128 breit),
in f32, Linux-Container mit 8 Kernen:

| State | Spitze über Start |
|---|---|
| 512 | 60 MB |
| 1.024 | 188 MB |
| 2.048 | 641 MB |
| 4.096 | 2.386 MB |

Pro Verdopplung etwa das Vierfache, denn Maske, Scores und Softmax sind
`L x L`. Hochgerechnet, nicht gemessen: 8.192 → ~9,5 GB, 65.536 → ~600 GB.
Schon das heutige Limit von 8.192 braucht also pro Schicht so viel Speicher,
wie dieser Container insgesamt hat.

Selbst wenn die Attention in Stücken gerechnet würde, bleibt bei 64k der
KV-Cache: kev-0.6b hält pro Token und Schicht 2 x 8 x 128 Werte. Über 28
Schichten sind das in f32 ~229 KB pro Token, also ~15 GB für einen
65.536-Token-State. Gerechnet, nicht gemessen.

## Was 65.536 hier überhaupt brächte

Von den 1.0-Checkpoints, die rkev laden kann (0.8B, 4B, 9B), ist laut ihren
Modellkarten nur ein Kontext von **8.192 Tokens** validiert. Die 65.536 sind
nur für Kev-27B validiert, und den unterstützt rkev nicht (Feature 05). Ein
höheres Limit würde also Antworten erlauben, die keine Messung deckt.

## Entscheidung und Ergebnis

Entschieden am 2026-10-30: **Das Limit bleibt bei 8.192**, die Attention wird in
Stücken gerechnet.

- `grouped_attention` rechnet höchstens 512 Query-Zeilen auf einmal
  (`QUERY_CHUNK`). Das ist exakt: Ein Unit-Test vergleicht gestückelt mit
  ungestückelt, und `a_state_scored_in_query_chunks_matches_the_transcription`
  prüft einen 600-Token-State gegen die Transkription, gepackt und über den
  Prefix. Mit falsch gestückelter Maske wird er genau an Token 512 rot.
- Gemessen, eine Attention-Schicht mit den Formen von kev-0.6b: 8.192 Tokens
  brauchen jetzt **1,03 GB** statt hochgerechneter ~9,5 GB, 4.096 Tokens 460 MB
  statt 2.386 MB.
- Quadratisch bleibt die dichte Maske (268 MB bei 8.192). Für 65.536 wäre
  sie 17 GB groß und müsste stückweise gebaut werden. Das ist nicht gemacht,
  weil das Limit bleibt.
- Die Abweichung zu Kev 1.0 steht in `docs/ablauf.md`, `docs/leistung.md` und
  `rkev/README.md`.
- Nicht gemacht: ein eigener Test auf dem Hybrid-Backbone (`qwen3_5.rs`). Seine
  Attention-Schichten rufen dieselbe `grouped_attention`, die der Unit-Test
  abdeckt. Ein Token-Budget für den Prefix-Cache war nur für 64k-States nötig.
