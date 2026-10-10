#!/usr/bin/env bash
# Builds (first time only, needs Rust: https://rustup.rs) and starts FretWright.
#   ./run.sh                 start the app
#   ./run.sh <project|song>  open a project folder, or start a new project from a song
#   ./run.sh cli ...         command line tool (fretwright-cli --help)
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust is needed to build FretWright from source: https://rustup.rs" >&2
    echo "(or download a ready-made release instead)" >&2
    exit 1
fi
if [ "${1:-}" = "cli" ]; then
    shift
    cargo build --release --quiet -p fw-cli
    exec target/release/fretwright-cli "$@"
fi
cargo build --release --quiet -p fw-app
exec target/release/fretwright "$@"
