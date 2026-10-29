#!/usr/bin/env bash
#
# Start the System One HTTP server (examples/serve.rs) in front of a checkpoint.
#
#   scripts/start-http.sh                              # ~/models/kev-0.6b on 127.0.0.1:8090
#   scripts/start-http.sh --checkpoint ~/models/kev-4b
#   scripts/start-http.sh --host 0.0.0.0 --port 8090   # reachable from outside, e.g. in Docker
#   RKEV_API_KEY=s3cret scripts/start-http.sh          # require Authorization: Bearer s3cret
#
#   --checkpoint <dir>  the Kev checkpoint (default <dir>/kev-0.6b, which is where
#                       scripts/local.sh --fetch jaredpalmer/kev-0.6b puts it)
#   --base <dir>        the base model, if not named by the checkpoint's
#                       adapter_config.json and already under <dir>
#   --dir <dir>         where the models are (default ~/models, or KEV_MODEL_DIR)
#   --host <addr>       default 127.0.0.1; 0.0.0.0 to accept other machines
#   --port <n>          default 8090
#   --device <name>     cpu (default) or metal; metal also adds --features metal,
#                       which only builds on macOS
#
# Anything else is handed to serve as it is: --api-key, --max-concurrent,
# --model, --dtype, --option-isolation ... (serve --help lists them).
# The model loads before the port opens; Ctrl-C stops the server.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$ROOT/rkev"
MODEL_DIR="${KEV_MODEL_DIR:-$HOME/models}"

checkpoint=""
base=""
host="127.0.0.1"
port="8090"
device=""
passthrough=()

while [ $# -gt 0 ]; do
    case "$1" in
        --checkpoint) checkpoint="${2:-}"; shift 2 ;;
        --base)       base="${2:-}"; shift 2 ;;
        --dir)        MODEL_DIR="${2:-}"; shift 2 ;;
        --host)       host="${2:-}"; shift 2 ;;
        --port)       port="${2:-}"; shift 2 ;;
        --device)     device="${2:-}"; shift 2 ;;
        -h|--help)    sed -n '3,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)            passthrough+=("$1"); shift ;;
    esac
done

# The crate is built from inside its own directory, where a relative path the
# caller gave would mean something else.
absolute() {
    case "${1:-}" in
        "") printf '' ;;
        /*) printf '%s' "$1" ;;
        *)  printf '%s/%s' "$PWD" "$1" ;;
    esac
}
MODEL_DIR="$(absolute "$MODEL_DIR")"
checkpoint="$(absolute "${checkpoint:-$MODEL_DIR/kev-0.6b}")"
base="$(absolute "$base")"

if [ ! -d "$checkpoint" ]; then
    echo "error: no checkpoint at $checkpoint" >&2
    echo "       fetch one with:  scripts/local.sh --fetch jaredpalmer/kev-0.6b" >&2
    echo "       or pass --checkpoint <dir>" >&2
    exit 1
fi

# A base beside the checkpoint, or named by its adapter_config.json - the same
# rule scripts/local.sh uses, so a --fetch there is enough here.
if [ -z "$base" ]; then
    if [ -f "$checkpoint/config.json" ] && [ ! -f "$checkpoint/adapter_config.json" ]; then
        base="$checkpoint"   # an already-merged checkpoint carries its own base
    elif [ -f "$checkpoint/adapter_config.json" ]; then
        repo="$(sed -n 's/.*"base_model_name_or_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
                "$checkpoint/adapter_config.json" | head -1)"
        repo="${repo%%@*}"
        base="$MODEL_DIR/$(echo "$repo" | tr 'A-Z/' 'a-z-')"
        if [ ! -d "$base" ]; then
            echo "error: $checkpoint adapts $repo, which is not at $base." >&2
            echo "       pass --base <dir>, or fetch both with scripts/local.sh --fetch" >&2
            exit 1
        fi
    else
        echo "error: $checkpoint has neither a config.json nor an adapter_config.json" >&2
        exit 1
    fi
fi
[ -f "$base/config.json" ] || { echo "error: no $base/config.json" >&2; exit 1; }

features="candle,server"
device_args=()
if [ -n "$device" ]; then
    device_args=(--device "$device")
    case "$device" in
        metal*) features="$features,metal" ;;
    esac
fi

# rustup installs into ~/.cargo but only a login shell picks that up.
if ! command -v cargo >/dev/null 2>&1 && [ -f "${CARGO_HOME:-$HOME/.cargo}/env" ]; then
    # shellcheck disable=SC1091
    . "${CARGO_HOME:-$HOME/.cargo}/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is not on PATH - no Rust toolchain in this environment." >&2
    exit 127
fi

cd "$CRATE_DIR"
echo "==> building serve (release, features $features)" >&2
cargo build --release --features "$features" --example serve

# Where cargo put it: the target dir can be moved with CARGO_TARGET_DIR.
target_dir="${CARGO_TARGET_DIR:-$CRATE_DIR/target}"
echo "==> $checkpoint over $base, on http://$host:$port" >&2
# A merged checkpoint is its own base and has no adapter for --checkpoint to load.
model_args=(--base "$base")
[ "$base" != "$checkpoint" ] && model_args+=(--checkpoint "$checkpoint")
# exec, so Ctrl-C and a container's stop signal reach the server itself.
exec "$target_dir/release/examples/serve" \
    "${model_args[@]}" --listen "$host:$port" \
    ${device_args[@]+"${device_args[@]}"} ${passthrough[@]+"${passthrough[@]}"}
