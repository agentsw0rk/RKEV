# Als HTTP-Server

RKev beantwortet System-One-Anfragen auch über HTTP, im selben Wire-Format wie
TypeSafes API. Ein aus dem Agent Modeler exportierter Agent läuft dann ohne
Code-Änderung gegen RKev statt gegen TypeSafe: `RKEV_BASE_URL` auf den Server zeigen
lassen, sonst nichts.

Gebaut ist das als Modul der Bibliothek (`rkev::server`, Feature `server`), davor
steht `examples/serve.rs` als fertiges Programm und davor `scripts/start-http.sh`.

## Starten

```bash
scripts/local.sh --fetch jaredpalmer/kev-0.6b   # einmal: Checkpoint und Basis holen
scripts/start-http.sh                           # ~/models/kev-0.6b auf 127.0.0.1:8090
```

Das Skript findet das Basismodell neben dem Checkpoint oder über dessen
`adapter_config.json` unter `~/models` (dort, wo `scripts/local.sh --fetch` es
ablegt), baut `serve` im Release-Modus und startet es. Das Modell lädt, bevor der
Port aufgeht: wer sich verbinden kann, bekommt auch eine Antwort. Ctrl-C beendet
den Server, ein `SIGTERM` (etwa von `docker stop`) ebenso.

| Option | Standard | Wofür |
|---|---|---|
| `--checkpoint <dir>` | `~/models/kev-0.6b` | der Kev-Checkpoint |
| `--base <dir>` | aus `adapter_config.json` | das Basismodell, wenn es woanders liegt |
| `--dir <dir>` | `~/models` (oder `KEV_MODEL_DIR`) | wo die Modelle liegen |
| `--host <addr>` | `127.0.0.1` | `0.0.0.0`, damit andere Rechner oder Container ihn erreichen |
| `--port <n>` | `8090` | |
| `--device cpu\|metal` | `cpu` | `metal` baut nur auf macOS |
| `--api-key <key>` | `RKEV_API_KEY`, sonst keiner | verlangt `Authorization: Bearer <key>` |
| `--max-concurrent <n>` | `4` | mehr gleichzeitige Anfragen bekommen sofort 503; `0` = unbegrenzt |
| `--model <name>` | Verzeichnisname des Checkpoints | der Name, den eine Anfrage neben `kev-latest` nennen darf |

Alles, was das Skript nicht kennt, reicht es an `serve` weiter (`--dtype`,
`--option-isolation`, …; `serve --help` listet sie). Ohne das Skript:

```bash
cd rkev
cargo run --release --features candle,server --example serve -- \
    --base ~/models/qwen-qwen3-0.6b-base --checkpoint ~/models/kev-0.6b \
    --listen 0.0.0.0:8090
```

## Den exportierten Agent anschließen

Der generierte Client (`SystemOneDecisions.kt`) liest zwei Umgebungsvariablen:

| Variable | Wert |
|---|---|
| `RKEV_BASE_URL` | Pflicht, z.B. `http://rkev:8090`; ein `/` am Ende wird abgeschnitten |
| `RKEV_API_KEY` | optional; gesetzt, schickt er `Authorization: Bearer <key>` mit |

Beide Fälle passen: ohne Key am Server nimmt er Anfragen ohne Header an, mit Key
verlangt er ihn. Jeder Entscheidungs-Node schickt pro Ausführung eine Anfrage mit
einer Frage unter der Id `decision` und verzweigt nach der Antwort. Jeder Status
außer 200 bricht den Agent-Lauf ab, ein stilles Default-Routing gibt es nicht.

## Anfrage und Antwort

```bash
curl -s localhost:8090/v1/systemone -H 'Content-Type: application/json' -d '{
  "state": "Paket kam nie an.",
  "questions": { "decision": { "type": "choice", "instructions": "Welches Team?",
    "criteria": { "returns": "Rueckgabe", "shipping": "Lieferung" } } } }'
```

Die Antwort hat die Form von `SystemOneResponse`: `model`, unter `answers` pro
Frage-Id die Antwort mit ihrem `type`, dazu `usage` und `latency_ms`. Bei `choice`
ist `choice` einer der gesendeten Optionsnamen, genau wie gesendet, und
`probabilities` kommt in der Reihenfolge der Optionen zurück. Die Antwortzeit ist
die eines Forward-Passes: für die drei Fragen aus dem Schnellstart gemessen 319 ms
auf Metal und 1671 ms auf der CPU, mit `decide`. Über HTTP gegen einen echten
Checkpoint ist noch nichts gemessen.

| Route | |
|---|---|
| `POST /v1/systemone` | die Antworten, alle Fragen in einem Pass |
| `POST /v1/systemone/separate` | jede Frage in ihrem eigenen Pass |
| `GET /v1/models` | die Namen, die `model` nennen darf: `{"models": [...]}` |
| `GET /health` | `{"status": "ok"}`, auch ohne Key erreichbar |
| `GET /` | eine leere Seite mit 200, auch ohne Key; für Load Balancer und Browser |

`model` darf fehlen (dann antwortet der geladene Checkpoint), `kev-latest` oder
der Name des geladenen Checkpoints sein. Alles andere ist ein 404.

## Fehler

Ein Fehler ist nie eine Antwort, sondern ein Status mit einem lesbaren Body. Der
Client gibt Status und Body in seiner Fehlermeldung weiter.

```json
{ "error": { "type": "invalid_request", "message": "invalid request: question \"decision\" has 0 options; it needs 1 to 255" } }
```

| Status | `type` | Wann |
|---|---|---|
| 400 | `invalid_request` | kein JSON, keine System-One-Anfrage, unbekannter Fragetyp, 0 oder mehr als 255 Optionen oder Stufen |
| 401 | `unauthorized` | am Server ist ein Key gesetzt, und der Header fehlt oder passt nicht |
| 404 | `model_not_found` | `model` nennt einen Namen, den der Server nicht bedient |
| 413 | `state_too_long` | die Anfrage passt nicht ins Kontextfenster des Modells; ein zu langer State wird abgelehnt, nicht gekürzt, und die Meldung nennt Länge und Limit (`kev.serve` antwortet hier mit 422) |
| 500 | `internal` | das Modell ist gescheitert |
| 503 | `unavailable` | alle Plätze belegt (`--max-concurrent`) |

503 kommt sofort und nicht nach einer Warteschlange: der Client gibt nach 60 s auf,
eine Antwort danach käme bei niemandem an. Bei vier Plätzen und 1,7 s pro Anfrage
auf einer CPU bleibt die längste Wartezeit weit darunter.

Eine ungültige Anfrage legt den Server nicht lahm. Das war nicht immer so: ein
`choice` ohne Optionen ließ früher den Forward-Pass abstürzen, und danach
antwortete der Server auf jede Anfrage mit 500 bis zum Neustart. Jetzt wird so
etwas vorher mit 400 abgelehnt.

## Prüfen

```bash
scripts/test-http.sh                             # gegen http://127.0.0.1:8090
RKEV_API_KEY=<key> scripts/test-http.sh --url http://rkev:8090
scripts/test-http.sh --strict                    # auch den Inhalt als Fehler werten
```

`test-http.sh` prüft einen laufenden Server mit `curl` und `jq` gegen die
Akzeptanztests des API-Dokuments: die drei Fragetypen, Optionsnamen mit
Leerzeichen, Umlauten und `$`, die Reihenfolge der Optionen, `Upgrade: h2c`, jeden
Fehlerstatus mit seinem Body, die 401 mit Key, und ob der Server nach den
kaputten Anfragen noch antwortet. Ob „Paket kam nie an.“ bei `shipping` landet,
braucht einen trainierten Checkpoint; diese Prüfungen warnen deshalb nur, außer
mit `--strict`. Bei einem Fehler endet es mit Exit-Code 1 und einer Liste.

## In ein eigenes Programm einbauen

```toml
[dependencies]
rkev = { path = "rkev", features = ["server"] }
```

```rust
use rkev::server::Server;

let server = Server::new(engine)                 // LocalEngine oder jedes SystemOne
    .model_name("kev-0.6b")                      // neben kev-latest
    .api_key(std::env::var("RKEV_API_KEY").ok()) // None: kein Header nötig
    .max_concurrent(4);                          // der Rest bekommt sofort 503
server.serve(tokio::net::TcpListener::bind("0.0.0.0:8090").await?).await?;
// oder server.router(), um die Routen in eine eigene axum-App zu hängen
```

Der Server hängt nur an der `SystemOne`-Naht, nicht an `LocalEngine`: er braucht
kein Modell und lässt sich so auch vor eine Aufzeichnung oder ein anderes Backend
stellen. Das Feature hebt die MSRV auf die von axum 0.8, 1.80.

## Was Annahme ist

- Ob TypeSafes eigener Server genau diese Statuscodes und Fehlertypen benutzt, ist
  nicht belegt; sie folgen dem API-Dokument für den Export. 400 statt der 422 von
  `kev.serve` ist bewusst so.
- Welche Form `jev-java` von `GET /v1/models` erwartet, ist nirgends beschrieben;
  `{"models": [...]}` ist die kleinste, die die Liste trägt.
- Nur HTTP/1.1. Javas `HttpClient` bietet bei `http://`-URLs ein Upgrade auf h2c
  an; der Server ignoriert das und antwortet in HTTP/1.1, und genau das braucht der
  Client.
- `/v1/systemone/permute` fehlt noch: es gehört nicht zur `SystemOne`-Naht,
  sondern nur zu `LocalEngine`.

Geprüft ist der Server offline (eigene Tests über einen echten TCP-Socket, ohne
Modell) und von Hand gegen einen Spielzeug-Checkpoint. Gegen einen echten
Checkpoint und einen exportierten Agent lief er noch nicht.
