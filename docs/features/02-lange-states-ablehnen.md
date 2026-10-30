# 02 — Zu lange States ablehnen statt still kürzen

**Status:** umgesetzt auf `feature/refuse-long-states` (2026-10-30), noch nicht gemergt
**Upstream:** [`2ea5660`](https://github.com/jaredpalmer/kev/commit/2ea5660) (2026-09-30),
`kev/model.py` (`ContextOverflow`, `encode`, `admit`) und `kev/serve.py`
(`Server.submit`, `_body`, `truncation_marks`, `/v1/models`)

## Problem

`rkev/src/encode.rs` (`encode`) kürzt einen State, der über
`Limits::max_state` liegt, ohne etwas zu sagen. Der Test
`a_state_longer_than_the_context_is_truncated_rather_than_refused` in
`tests/local_engine.rs` legt genau dieses Verhalten fest. Das Modell beantwortet
dann eine Frage über den Anfang eines Dokuments, und der Aufrufer hält das für
eine Antwort über das ganze.

Upstream lehnt das seit 2026-09-30 ab. Kürzen geht dort nur noch auf
ausdrücklichen Wunsch, und dann sagt jede Antwort, ob gekürzt wurde:

- `ContextOverflow` trägt `state_tokens` (die Länge des States, `<state>`
  mitgezählt) und `max_state` (das Limit).
- Die Meldung lautet `state is 70,000 tokens, over the 65,536-token limit (the
  <state> token included): shorten the document or split it across requests`,
  dazu der Hinweis auf `KEV_TRUNCATE_STATES=1`.
- Mit `KEV_TRUNCATE_STATES=1` trägt **jede** Antwort `truncated: bool` sowie
  `usage.state_tokens` und `usage.state_tokens_used`. Ohne den Schalter bleibt
  der Body so, wie er war.
- `/v1/systemone/separate` und `/permute` übernehmen diese Marken.
- `/v1/models` meldet `max_state_tokens` und `truncate_states`.
- Ein State von genau `max_state` Tokens (mit `<state>`) wird noch angenommen.
- Ein Fragezweig über `max_branch` ist weiterhin immer ein Fehler.

## Umsetzung

Engine (`encode.rs`, `local.rs`, `error.rs`):

1. Test umkehren: Ein zu langer State ergibt `Error::ContextOverflow`, und
   `is_validation()` ist wahr. Die Meldung nennt Länge und Limit.
2. Grenzfall testen: genau `max_state` Tokens werden ganz gelesen und nicht
   abgelehnt.
3. Opt-in testen: Mit `LocalEngine::with_truncated_states(true)` (Name noch
   offen) wird gekürzt, und die Antwort sagt es.
4. Wo die Marken im Typ leben: `Response` bekommt `truncated: Option<bool>`, und
   `Usage` bekommt `state_tokens` / `state_tokens_used` als `Option<u64>`, alle
   mit `#[serde(default, skip_serializing_if = "Option::is_none")]`. Damit
   bleibt der Body ohne Opt-in unverändert, und eine Aufnahme ohne die Felder
   lässt sich weiter lesen (Invariante „Optional response fields“).
   `tests/wire_format.rs` pinnt beides.
5. Den 422-Hinweis auf `KEV_TRUNCATE_STATES` übersetzen wir in unseren Schalter.
   Der Text sonst wie upstream.

Server (`src/server.rs`, `examples/serve.rs`, `scripts/start-http.sh`):

6. Der Server bildet `ContextOverflow` heute auf **413 `state_too_long`** ab,
   nicht auf 422. Das gibt das API-Dokument des Users vor („RKEV HTTP-API
   (System One kompatibel)“), und `docs/server.md` und `rkev/README.md` sagen es
   auch. **Entscheidung nötig:** bei 413 bleiben (empfohlen; nur die Meldung
   wird jetzt auch bei zu langen States geliefert statt eines stillen Kürzens),
   oder auf 422 wie `kev.serve` wechseln.
7. Schalter am Server: `Server::truncate_states(bool)` und in
   `examples/serve.rs` `--truncate-states` bzw. `RKEV_TRUNCATE_STATES=1`.
8. `/v1/models`: Upstream liefert eine Modellkarte, wir nur `{"models": [...]}`.
   `max_state_tokens` und `truncate_states` dort ergänzen, ohne die Liste zu
   ändern, die `jev-java` liest. Abgedeckt in `tests/server.rs`.
9. `/separate` und `/permute` (falls der Server sie anbietet) übernehmen die
   Marken wie `truncation_marks`.

Doku:

10. `docs/ablauf.md` („Der State wird auf 8192 Tokens gekürzt“), das Root-
    `README.md` (*Vom Ticket zur Antwort*), `rkev/README.md`, `docs/server.md`
    (Fehlertabelle, Schalter) und die Docs von `Limits` in `encode.rs`.
11. `CLAUDE.md`: unter *Invariants* festhalten, dass ein State nie still gekürzt
    wird.

## Fertig, wenn

- ohne Schalter kein Pfad mehr kürzt (Engine, Batch, Prefix, Server),
- mit Schalter jede Antwort `truncated` trägt,
- der Body ohne Schalter Byte für Byte gleich bleibt (`tests/wire_format.rs`),
- die 413/422-Entscheidung getroffen und dokumentiert ist.

## Ergebnis

- **Ablehnen:** Ein State über `max_state` ist `Error::ContextOverflow` mit
  Upstreams Meldung (`state is N tokens, over the M-token limit (the <state>
  token included): …`). Das gilt auf jedem Pfad, weil alle über `encode`
  laufen: gepackt, getrennt, Batch, Permute. Die Grenze (genau `max_state`) ist
  getestet, ebenso ein absichtlicher Off-by-one.
- **413 bleibt**, wie im API-Dokument. Die Abweichung zu `kev.serve` (422) steht
  in `docs/server.md` und `rkev/README.md`.
- **Opt-in:** `LocalEngine::with_truncated_states(true)` bzw.
  `serve --truncate-states` / `RKEV_TRUNCATE_STATES=1`. Dann tragen alle
  Antworten `truncated` und `usage.state_tokens` / `state_tokens_used`,
  auch `/separate` und der `permute`-Umschlag.
- **Wire-Format:** Ohne Opt-in bleibt der Body unverändert. Das hält
  `a_response_serialises_back_into_the_readme_shape` fest. Der Body eines
  kürzenden `kev.serve` übersteht einen Roundtrip.
- **API-Änderung:** `SystemOneResponse` hat das neue Feld `truncated`. Code,
  der die Struktur als Literal baut, muss es ergänzen. `Usage` hat zwei neue
  `Option`-Felder, `Usage::default()` deckt sie ab.
- **Nicht umgesetzt: `/v1/models`.** Upstream liefert dort eine Modellkarte mit
  `max_state_tokens` und `truncate_states`, wir nur `{"models": [...]}`, so
  wie es das API-Dokument verlangt. Der Server ist generisch über den
  `SystemOne`-Seam und kennt die Limits der Engine nicht. Dafür müsste der Seam
  sie preisgeben, oder der Server bekäme sie doppelt konfiguriert. Erst
  angehen, wenn ein Client die Felder braucht.
- Die Invariante steht in `CLAUDE.md`. Die Datei ist in `.gitignore`, die
  Änderung ist also nur lokal.
