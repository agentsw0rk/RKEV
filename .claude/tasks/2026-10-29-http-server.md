# 2026-10-29-http-server.md

HTTP-Server für rkev, kompatibel zu System One, als Modul der Bibliothek.
Spezifikation: Claude-Docs-Dokument „RKEV HTTP-API (System One kompatibel)“,
https://claude.ai/artifact/Q7UAmh6zaDz5CbnCx74nBR (Tab 649329f1-dd5e).
Aufrufer ist `SystemOneDecisions.kt` aus dem Agent Modeler (Feature 18): eine
Frage pro Request, ID immer `decision`, `state` als String, Timeout 60 s.

## Progress
- ✅ Spezifikation gelesen, Entwurf mit dem Nutzer abgestimmt
- ✅ Implementierung auf `feature/http-server`, TDD, 26 Commits, alle Checks grün
  (fmt, clippy, Feature-Matrix inkl. `--no-default-features --features server`).
  116 Tests (main: 96).
- ✅ End-to-end von Hand: `serve` gegen den Spielzeug-Checkpoint aus den
  qwen3-Tests (`/tmp/kev-qwen3-answers`), mit curl jede Route, Auth, h2c, alle
  Fehlercodes.
- ⏸ **Push und Draft-PR offen:** `gh` ist seit dem Container-Neustart nicht
  angemeldet (`gh auth login` durch den Nutzer).
- ⏸ Gegen einen echten Checkpoint und einen exportierten Graph-Agent noch nicht
  gelaufen (letzter Punkt der Akzeptanztests im Dokument).

## Was gebaut ist
- `rkev::server::Server<B: SystemOne + Clone + Send + Sync + 'static>`:
  `.model_name()`, `.api_key(Option<String>)`, `.max_concurrent(n)`,
  `.router()` (axum, einbettbar), `.serve(TcpListener)`.
- Routen: `POST /v1/systemone`, `POST /v1/systemone/separate`,
  `GET /v1/models` (`{"models": [...]}`, mit Key geschützt), `GET /health`
  (`{"status":"ok"}`, immer offen).
- Fehler → `{"error":{"type","message"}}`: 400 `invalid_request` (kaputtes JSON,
  `Error::Invalid`), 401 `unauthorized`, 404 `model_not_found`, 413
  `state_too_long` (`ContextOverflow`), 500 `internal`, 503 `unavailable`.
- `examples/serve.rs`: `--listen` (Standard 127.0.0.1:8090), `--api-key` /
  `RKEV_API_KEY`, `--max-concurrent` (Standard 4, 0 = unbegrenzt), `--model`
  (Standard: Verzeichnisname des Checkpoints).
- Antworttypen sind jetzt `Serialize`; `latency_ms` fehlt statt `null`.

## Key Decisions (vom Nutzer bestätigt)
- **Validierungsfehler → 400**, wie im Dokument (nicht 422 wie kev.serve).
- **`model`:** `kev-latest` plus konfigurierte Namen, sonst 404; ohne Feld → Default.
- **TDD** mit einem Commit pro Zyklus.

## Eigene Entscheidungen (im PR erwähnen)
- axum 0.8 → MSRV 1.80 nur für das Feature `server` (wie candle es schon macht).
- 503 sofort statt Warteschlange: der Client bricht nach 60 s ohnehin ab.
- `/v1/models`-Format ist eine Annahme; was `jev-java` dort liest, ist nicht belegt.

## Gefunden und behoben: Engine-Bug
- `choice` ohne Optionen / `score` ohne Stufen paniken in `readout.rs`; die Panik
  vergiftet den Mutex des Backends, danach antwortete **jede** Anfrage mit 500
  bis zum Neustart. Jetzt `validate()` in `local.rs` vor dem Lock in allen drei
  Einstiegen: 1..=255 Optionen/Stufen, sonst `Error::Invalid`.
- Restrisiko: jede *andere* Panik im Forward-Pass vergiftet den Mutex genauso.
  Nicht angefasst (Wiederherstellung nach Poison wäre eine Design-Entscheidung).

## Next Steps
1. Nutzer: `gh auth login`, dann `git push -u origin feature/http-server` und
   Draft-PR (`/pr-draft`).
2. Gegen `kev-0.6b` starten und einen exportierten Agent mit
   `RKEV_BASE_URL=http://<host>:8090` laufen lassen.
3. Optional: `/v1/systemone/permute` (geht nicht über den `SystemOne`-Trait,
   nur `LocalEngine::permute`).

## Gotchas
- **Umgebung nach Neustart neu aufbauen:** rustup (minimal + rustfmt, clippy),
  dann der Shim; `. ~/rkev-env.sh` setzt PATH, RUSTFLAGS, RUSTC_WRAPPER und
  `CARGO_TARGET_DIR=$HOME/target-rkev`. Der Shim darf `crt1.o` und
  `-dynamic-linker` **nicht** an Proc-Macros (`--crate-type proc-macro`)
  hängen, sonst „can't find crate for *_derive“. Ohne `. ~/rkev-env.sh` in
  derselben Shell: „linker `cc` not found“.
- **target/ nicht auf dem Mac-Mount** (`/workspace` = `/run/host_mark/Users`):
  dort kamen sporadisch „can't find crate“-Fehler. Deshalb `CARGO_TARGET_DIR`.
- **Commit nur, wenn `scripts/test.sh` mit Exit 0 endet** — nicht auf `grep`
  verlassen (ein Commit mit Format-Fehler ist so durchgerutscht und wurde amended).
- `CLAUDE.md` und `Cargo.lock` sind in `.gitignore` — nicht committen.
- `sed -i` auf `scripts/test.sh` löscht das Exec-Bit; danach `chmod +x`.
- Kein `python3`, kein `sudo` im Container.
- Zwei rustdoc-Warnungen ohne `local` (`lib.rs:58`, `system_one.rs:13`) sind alt.
