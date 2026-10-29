#!/usr/bin/env bash
#
# Check a running System One HTTP server against what the Agent Modeler's
# exported client needs: the acceptance tests of "RKEV HTTP-API (System One
# kompatibel)", over the wire, with curl.
#
#   scripts/test-http.sh                              # http://127.0.0.1:8090
#   scripts/test-http.sh --url http://rkev:8090
#   RKEV_API_KEY=s3cret scripts/test-http.sh          # a server started with a key
#   scripts/test-http.sh --strict                     # content checks fail the run too
#
#   --url <url>      the server's base URL (default RKEV_BASE_URL, else
#                    http://127.0.0.1:8090); a trailing / is dropped, as the
#                    exported client does
#   --api-key <key>  sent as Authorization: Bearer <key> (default RKEV_API_KEY);
#                    with one, the script also checks that a request without it
#                    is refused
#   --strict         the content checks (does "Paket kam nie an." go to
#                    shipping?) fail the run instead of only warning; they need
#                    a trained checkpoint, a toy or untrained one answers at chance
#
# Shape checks fail the run: status codes, answer types, that a choice is one
# of the options sent, option order, the error body. Start a server with
# scripts/start-http.sh first. Needs curl and jq.

set -uo pipefail

url="${RKEV_BASE_URL:-http://127.0.0.1:8090}"
key="${RKEV_API_KEY:-}"
strict=0

while [ $# -gt 0 ]; do
    case "$1" in
        --url)     url="${2:-}"; shift 2 ;;
        --api-key) key="${2:-}"; shift 2 ;;
        --strict)  strict=1; shift ;;
        -h|--help) sed -n '3,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)         echo "error: unknown argument $1 (try --help)" >&2; exit 1 ;;
    esac
done
url="${url%/}"

for tool in curl jq; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is not installed" >&2; exit 127; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

passed=0
failed=0
warned=0
failures=""

ok()   { passed=$((passed + 1)); printf '  ok    %s\n' "$1"; }
fail() { failed=$((failed + 1)); failures="${failures}\n  - $1"; printf '  FAIL  %s\n' "$1"; [ -n "${2:-}" ] && printf '        %s\n' "$2"; }
warn() {
    if [ "$strict" -eq 1 ]; then fail "$1" "${2:-}"; return; fi
    warned=$((warned + 1)); printf '  warn  %s\n' "$1"; [ -n "${2:-}" ] && printf '        %s\n' "$2"
}

# One request. Sets $status, $seconds and $body (the reply, as text).
#   call METHOD PATH [BODY] [extra curl arguments...]
call() {
    local method="$1" path="$2" data="${3:-}"
    shift 3 2>/dev/null || shift $#
    local args=(-s -X "$method" -o "$work/body" -w '%{http_code} %{time_total}'
                --connect-timeout 10 --max-time 60 -H 'Content-Type: application/json')
    [ -n "$data" ] && args+=(--data-binary "$data")
    local out
    out="$(curl "${args[@]}" "$@" "$url$path")" || out="000 0"
    status="${out%% *}"
    seconds="${out##* }"
    body="$(cat "$work/body" 2>/dev/null)"
}

# The key, when there is one; every API request carries it, as the client does.
auth=()
[ -n "$key" ] && auth=(-H "Authorization: Bearer $key")

# Check a jq expression against the last body: expect PATH EXPRESSION DESCRIPTION
expect() {
    if printf '%s' "$body" | jq -e "$1" >/dev/null 2>&1; then
        ok "$2"
    else
        fail "$2" "body: $(printf '%s' "$body" | head -c 300)"
    fi
}

ask() {
    call POST /v1/systemone "$1" ${auth[@]+"${auth[@]}"} "${@:2}"
}

echo "==> $url"

# --- reachable at all --------------------------------------------------------
echo "--- the server"
call GET /health ""
if [ "$status" = "000" ]; then
    echo "error: no server at $url - start one with scripts/start-http.sh" >&2
    exit 1
fi
[ "$status" = 200 ] && ok "GET /health is 200" || fail "GET /health is 200" "got $status: $body"

call GET / ""
[ "$status" = 200 ] && [ -z "$body" ] && ok "GET / is an empty 200" \
    || fail "GET / is an empty 200" "got $status: $(printf '%s' "$body" | head -c 200)"

call GET /v1/models "" ${auth[@]+"${auth[@]}"}
if [ "$status" = 200 ]; then
    expect '.models | index("kev-latest") != null' "GET /v1/models lists kev-latest"
    models="$(printf '%s' "$body" | jq -r '.models | join(", ")' 2>/dev/null)"
else
    fail "GET /v1/models is 200" "got $status: $body"
    models=""
fi

# --- the three request examples from the document ----------------------------
echo "--- the three question types"
choice='{"state":"Schuhe zwei Wochen zu spaet und in der falschen Groesse.","questions":{"decision":{"type":"choice","instructions":"Welches Team soll das uebernehmen?","criteria":{"returns":"Umtausch, Rueckgabe, falsche Ware","shipping":null}}}}'
ask "$choice"
if [ "$status" = 200 ]; then
    ok "choice is 200 (${seconds}s)"
    expect '.answers.decision.type == "choice"' "choice answers with type choice"
    expect '.answers.decision.choice | IN("returns", "shipping")' "choice is one of the options sent"
    expect '(.answers.decision.probabilities | keys_unsorted) == ["returns", "shipping"]' \
        "probabilities keep the order the options were sent in"
    expect '.model | type == "string"' "the response names a model"
else
    fail "choice is 200" "got $status: $body"
fi

ask '{"state":"Kunde droht mit Anwalt.","model":"kev-latest","questions":{"decision":{"type":"noul","instructions":"Braucht das dringend einen Menschen?"}}}'
if [ "$status" = 200 ]; then
    ok "noul with model kev-latest is 200 (${seconds}s)"
    expect '.answers.decision.type == "noul" and (.answers.decision.noul | . >= 0 and . <= 1)' \
        "noul is a probability between 0 and 1"
else
    fail "noul with model kev-latest is 200" "got $status: $body"
fi

ask '{"state":"Das ist jetzt das dritte Mal!","questions":{"decision":{"type":"score","instructions":"Wie veraergert ist der Kunde?","criteria":["Ruhig","Veraergert","Sehr wuetend"]}}}'
if [ "$status" = 200 ]; then
    ok "score is 200 (${seconds}s)"
    expect '.answers.decision.type == "score" and (.answers.decision.score | . >= 0 and . <= 2)' \
        "score is a level index between 0 and 2"
    expect '.answers.decision.legend == {"0":"Ruhig","1":"Veraergert","2":"Sehr wuetend"}' \
        "score's legend names the levels sent"
else
    fail "score is 200" "got $status: $body"
fi

# --- what the client relies on -------------------------------------------------
echo "--- what the exported client relies on"
odd='{"state":"Bitte an die Buchhaltung.","questions":{"decision":{"type":"choice","instructions":"Wohin?","criteria":{"Rück gabe":"Umtausch","Buch$haltung & Co":"Rechnungen","Ärger/Eskalation":null}}}}'
ask "$odd"
if [ "$status" = 200 ]; then
    expect '.answers.decision.choice | IN("Rück gabe", "Buch$haltung & Co", "Ärger/Eskalation")' \
        "an option name with spaces, umlauts and \$ comes back exactly as sent"
else
    fail "options with spaces, umlauts and \$ are 200" "got $status: $body"
fi

ordered='{"state":"Paket kam nie an.","questions":{"decision":{"type":"choice","instructions":"Welches Team?","criteria":{"shipping":"Lieferung","returns":"Rueckgabe","billing":"Rechnung"}}}}'
ask "$ordered"
expect '(.answers.decision.probabilities | keys_unsorted) == ["shipping", "returns", "billing"]' \
    "a different option order comes back in that order"

ask "$choice" --http1.1 -H 'Connection: Upgrade, HTTP2-Settings' -H 'Upgrade: h2c' \
    -H 'HTTP2-Settings: AAMAAABkAAQAoAAAAAIAAAAA'
[ "$status" = 200 ] && ok "an h2c upgrade offer (Java's HttpClient) is answered with 200" \
    || fail "an h2c upgrade offer is answered with 200" "got $status: $body"

# --- the errors ----------------------------------------------------------------
echo "--- errors are a status and a readable body"
error_is() {   # error_is STATUS TYPE DESCRIPTION
    if [ "$status" = "$1" ] && printf '%s' "$body" | jq -e --arg t "$2" \
         '.error.type == $t and (.error.message | type == "string" and length > 0)' >/dev/null 2>&1; then
        ok "$3: $1 $2"
    else
        fail "$3: $1 $2" "got $status: $(printf '%s' "$body" | head -c 300)"
    fi
}

ask '{"state": "abgeschnitten'
error_is 400 invalid_request "broken JSON"

ask '{"state":"x","questions":{"decision":{"type":"choice","instructions":"?","criteria":{}}}}'
error_is 400 invalid_request "a choice without options"

ask '{"state":"x","questions":{"decision":{"type":"maybe","instructions":"?"}}}'
error_is 400 invalid_request "an unknown question type"

ask '{"state":"x","model":"kein-solches-modell","questions":{"decision":{"type":"noul","instructions":"?"}}}'
error_is 404 model_not_found "an unknown model"

if [ -n "$key" ]; then
    call POST /v1/systemone "$choice"
    error_is 401 unauthorized "no Authorization header"
    call POST /v1/systemone "$choice" -H "Authorization: Bearer ${key}x"
    error_is 401 unauthorized "a wrong key"
else
    printf '  skip  the 401 checks: no key given (--api-key or RKEV_API_KEY)\n'
fi

# The server must still answer after all of that: a bad request once poisoned
# the engine for every request after it.
ask "$choice"
[ "$status" = 200 ] && ok "still answers after the bad requests" \
    || fail "still answers after the bad requests" "got $status: $body"

# --- content: needs a trained checkpoint ---------------------------------------
echo "--- content (needs a trained checkpoint; --strict makes these fail)"
ask '{"state":"Paket kam nie an.","questions":{"decision":{"type":"choice","instructions":"Welches Team?","criteria":{"returns":"Rueckgabe","shipping":"Lieferung"}}}}'
got="$(printf '%s' "$body" | jq -r '.answers.decision.choice // empty' 2>/dev/null)"
[ "$got" = shipping ] && ok "\"Paket kam nie an.\" goes to shipping" \
    || warn "\"Paket kam nie an.\" goes to shipping" "got ${got:-$status}"

ask '{"state":"Ich wurde zweimal abgebucht.","questions":{"decision":{"type":"choice","instructions":"Welches Team?","criteria":{"returns":"Rueckgabe","shipping":"Lieferung","billing":"Abbuchungen, Rechnungen"}}}}'
got="$(printf '%s' "$body" | jq -r '.answers.decision.choice // empty' 2>/dev/null)"
[ "$got" = billing ] && ok "\"Ich wurde zweimal abgebucht.\" goes to billing" \
    || warn "\"Ich wurde zweimal abgebucht.\" goes to billing" "got ${got:-$status}"

ask "$choice"
if awk -v s="$seconds" 'BEGIN { exit !(s < 60) }'; then
    ok "one question answered in ${seconds}s, under the client's 60 s"
else
    fail "one question answered under 60 s" "took ${seconds}s"
fi

# --- summary -------------------------------------------------------------------
echo ""
echo "==> $passed passed, $failed failed, $warned warnings${models:+ (models: $models)}"
if [ "$failed" -gt 0 ]; then
    printf "==> FAILED:$failures\n" >&2
    exit 1
fi
