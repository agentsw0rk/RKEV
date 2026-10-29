# 2026-10-29-http-server.md

HTTP-Server für rkev, kompatibel zu System One, als Modul der Bibliothek.
Spezifikation: Claude-Docs-Dokument „RKEV HTTP-API (System One kompatibel)“,
https://claude.ai/artifact/Q7UAmh6zaDz5CbnCx74nBR (Tab 649329f1-dd5e).
Aufrufer ist `SystemOneDecisions.kt` aus dem Agent Modeler (Feature 18): eine
Frage pro Request, ID immer `decision`, `state` als String, Timeout 60 s.

## Progress
- ✅ Spezifikation gelesen, Entwurf mit dem Nutzer abgestimmt
- 🔄 Branch `feature/http-server`, TDD, lokal committet (Push offen: `gh` nach
  dem Neustart nicht angemeldet). Fertig:
  - Antworttypen `Serialize`, `latency_ms` weggelassen wenn `None`
  - `rkev::server::Server`, Feature `server`, `POST /v1/systemone` → 200
  - `Error::Invalid` → 400 `invalid_request`; kaputtes JSON → 400 im selben Format
  - `.api_key(Some(..))` → 401 `unauthorized` ohne/mit falschem Key
  - `Upgrade: h2c` wird ignoriert (200 über HTTP/1.1)
  - Feature-Matrix in `scripts/test.sh` um `server` erweitert

## Key Decisions (vom Nutzer bestätigt)
- **Validierungsfehler → 400** `invalid_request`, wie im Dokument (nicht 422
  wie kev.serve; das ist eine bewusste Abweichung von der Referenz).
- **Feld `model`:** der geladene Name plus konfigurierbare Aliase (Default
  `kev-latest`) werden akzeptiert, alles andere → 404 `model_not_found`;
  fehlt das Feld → Default-Modell.
- **TDD:** `/tdd`-Modus, ein Akzeptanztest nach dem anderen, je ein Commit,
  Branch `feature/http-server`, Draft-PR am Ende.

## Entwurf
- Neues Feature `server` (axum + hyper, reines Rust ohne C). Hängt **nur** an
  der Schnittstelle `SystemOne`, nicht an `local`/`candle`, damit es ohne Modell
  über den Stub-Backend aus `tests/fixtures` testbar ist.
- `src/server.rs`: `Server::new(engine: S) where S: SystemOne + Clone + Send + Sync + 'static`,
  `.model_name()`, `.aliases()`, `.api_key(Option<String>)`, `.max_concurrent(n)`,
  `.router() -> axum::Router`; `serve(router, addr)`.
- Routen: `POST /v1/systemone` (Pflicht), `POST /v1/systemone/separate`,
  `GET /health`, `GET /v1/models`. `/permute` später, da `permute` nicht zur
  `SystemOne`-Schnittstelle gehört (nur `LocalEngine`).
- Fehler auf Status-Codes: `Error::Invalid` und kaputtes JSON → 400
  `invalid_request`; `ContextOverflow` → 413 `state_too_long`;
  `Engine`/`Model` → 500 `internal`; alle Plätze belegt → sofort 503
  `unavailable`; Auth fehlt oder falsch → 401 `unauthorized` (nur wenn ein Key
  gesetzt ist). Body: `{"error":{"type","message","field"}}`.
- `Upgrade: h2c` muss normal beantwortet werden; mit einem echten `TcpStream`
  testen (kein HTTP-Client als Abhängigkeit; der alte Client wurde auf Wunsch
  des Nutzers entfernt).
- `examples/serve.rs` (required-features candle + server): `--base --checkpoint --port --device`,
  `--api-key` / `RKEV_API_KEY`.
- `tests/server.rs`: die Akzeptanztests aus dem Dokument.
- Doku mitziehen: `rkev/README.md`, `//!` in `src/lib.rs`, `CLAUDE.md`,
  Root-`README.md` (Deutsch), Feature-Matrix in `scripts/test.sh` (+ `--features server`).

## Next Steps
1. Prüfen, ob die Shell läuft (`df -h /tmp`); Branch `feature/http-server` anlegen.
2. `/tdd` aktivieren. Erster Test: `POST /v1/systemone` mit einer `choice`-Frage
   → 200, `answers.decision.choice` ist einer der gesendeten Optionsnamen.
3. Dann der Reihe nach: noul/score, Optionsnamen mit Leerzeichen, Umlauten oder `$`,
   Reihenfolge, Auth an/aus, h2c, kaputtes JSON → 400, model-Aliase/404,
   413, 503, /health, /v1/models.
4. Offener Kommentar im Dokument (Thread 67f96290-3284, „Sollen die Fehlercodes
   so bleiben…?“) per Claude-Docs-Kommentar beantworten: das echte Format
   von System One ist nicht bekannt, die Codes bleiben als gekennzeichnete
   Annahme, 400 statt 422 ist bewusst so entschieden.

## Gotchas
- **Umgebung nach Neustart neu aufbauen:** rustup (minimal + rustfmt, clippy),
  dann der Shim; `. ~/rkev-env.sh` setzt PATH, RUSTFLAGS, RUSTC_WRAPPER und
  `CARGO_TARGET_DIR=$HOME/target-rkev`. Der Shim darf `crt1.o` und
  `-dynamic-linker` **nicht** an Proc-Macros (`--crate-type proc-macro`)
  hängen, sonst „can't find crate for *_derive“.
- **target/ nicht auf dem Mac-Mount** (`/workspace` = `/run/host_mark/Users`):
  dort kamen sporadisch „can't find crate“-Fehler. Deshalb `CARGO_TARGET_DIR`.
- `CLAUDE.md` und `Cargo.lock` sind in `.gitignore` — nicht committen.
- `sed -i` auf `scripts/test.sh` löscht das Exec-Bit; danach `chmod +x`.
- Kein `python3` im Container.
- Ohne C-Toolchain linkt `cargo` nur über den Shim (siehe CLAUDE.md, „Linking
  in a container with no C toolchain“). Nach dem Neustart prüfen, ob
  `~/bin/rustc-shim`, `~/lib-shim` und `~/crt/crt1.o` noch da sind.
- MSRV 1.75: die axum-Version passend wählen.
