#!/usr/bin/env bash
#
# Check scripts/local.sh --fetch without the network: a hub tree is laid out
# under a temporary directory and served through KEV_HF=file://..., which is
# what curl reads the same way it reads https.
#
#   scripts/test-fetch.sh
#
# Needs curl. Nothing is built and no model is loaded: every run uses
# --fetch-only.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

command -v curl >/dev/null 2>&1 || { echo "error: curl is not installed" >&2; exit 127; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

passed=0
failed=0
ok()   { passed=$((passed + 1)); printf '  ok    %s\n' "$1"; }
fail() { failed=$((failed + 1)); printf '  FAIL  %s\n' "$1"; [ -n "${2:-}" ] && printf '        %s\n' "$2"; }

# put <repo> <revision> <file> <content>: one file of the fake hub.
put() {
    mkdir -p "$work/hub/$1/resolve/$2/$(dirname "$3")"
    printf '%s' "$4" > "$work/hub/$1/resolve/$2/$3"
}

# A checkpoint at main and at v1.0, told apart by what their files say, and
# the base it names.
for revision in main v1.0; do
    put jaredpalmer/kev-test "$revision" adapter_config.json \
        '{"base_model_name_or_path": "Qwen/Test-Base", "r": 16}'
    put jaredpalmer/kev-test "$revision" adapter_model.safetensors "adapter at $revision"
    put jaredpalmer/kev-test "$revision" head.pt "head at $revision"
done
put Qwen/Test-Base main config.json '{}'
put Qwen/Test-Base main tokenizer.json '{}'
put Qwen/Test-Base main model.safetensors "base weights"

# fetch <models dir> <repo[@revision]>: local.sh --fetch-only, output kept.
fetch() {
    KEV_HF="file://$work/hub" "$ROOT/scripts/local.sh" --fetch-only \
        --dir "$1" --fetch "$2" > "$work/out" 2>&1
}

echo "==> --fetch-only"
if fetch "$work/plain" jaredpalmer/kev-test; then
    ok "fetches and stops"
else
    fail "fetches and stops" "$(tail -3 "$work/out")"
fi
if [ "$(cat "$work/plain/kev-test/head.pt" 2>/dev/null)" = "head at main" ]; then
    ok "the checkpoint is main's"
else
    fail "the checkpoint is main's"
fi
if [ -s "$work/plain/qwen-test-base/model.safetensors" ]; then
    ok "the base named in adapter_config.json came along"
else
    fail "the base named in adapter_config.json came along"
fi
if grep -q "offline suite\|cargo" "$work/out"; then
    fail "nothing is built or run after the fetch" "$(grep -m1 "offline suite\|cargo" "$work/out")"
else
    ok "nothing is built or run after the fetch"
fi

echo "==> a revision: repo@v1.0"
if fetch "$work/tagged" jaredpalmer/kev-test@v1.0; then
    ok "fetches a tagged revision"
else
    fail "fetches a tagged revision" "$(tail -3 "$work/out")"
fi
if [ "$(cat "$work/tagged/kev-test/head.pt" 2>/dev/null)" = "head at v1.0" ]; then
    ok "the checkpoint is v1.0's, not main's"
else
    fail "the checkpoint is v1.0's, not main's" "$(ls "$work/tagged" 2>&1)"
fi

echo ""
if [ "$failed" -gt 0 ]; then
    echo "==> FAILED: $failed of $((passed + failed))" >&2
    exit 1
fi
echo "==> all $passed checks passed"
