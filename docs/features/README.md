# Features: Abgleich mit Kev 1.0

Kev 1.0 (GitHub-Release [`kev-1.0`](https://github.com/jaredpalmer/kev/releases/tag/kev-1.0),
2026-10-01) trainiert nichts neu, versioniert aber die Checkpoints und den
Serving-Code als eine Familie. Seit unserem Port hat sich Upstream an Stellen
geändert, die unsere Antworten oder unser Verhalten direkt betreffen. Jede Datei
hier ist ein Feature und wird als eigener Branch und PR umgesetzt, in dieser
Reihenfolge:

| # | Feature | Warum zuerst | Status |
|---|---|---|---|
| 01 | [Score-Konfidenz nach Upstream](01-score-konfidenz.md) | ändert jede `score`-Antwort; klein | offen |
| 02 | [Zu lange States ablehnen statt still kürzen](02-lange-states-ablehnen.md) | stilles Kürzen liefert eine Antwort auf eine andere Frage | offen |
| 03 | [Kontextlimit 65.536 Tokens](03-kontextlimit-65k.md) | braucht 02; Speicherfrage auf der CPU offen | offen |
| 04 | [Checkpoints der 1.0-Familie holen und dokumentieren](04-kev-1.0-checkpoints.md) | Doku und `--fetch`, ändert keine Antworten | offen |
| 05 | [Kev-27B (volle Gewichte)](05-kev-27b.md) | zurückgestellt | zurückgestellt |

Für jedes Feature gilt: TDD (`/tdd`), ein Branch, ein PR, Status hier und in der
Feature-Datei nachziehen, sobald er gemergt ist.

Grundlage des Abgleichs (Stand 2026-10-30): die Release-Notes von `kev-1.0` und
die Commits an `kev/api.py`, `kev/model.py` und `kev/serve.py` seit 2026-09-15.
Die Upstream-Commits sind in jeder Datei mit Hash genannt, damit der Port sie
nachlesen kann, statt sich auf diese Zusammenfassung zu verlassen.
