#!/usr/bin/env bash

set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
vite_pid=""

cleanup() {
  if [[ -n "$vite_pid" ]] && kill -0 "$vite_pid" 2>/dev/null; then
    kill "$vite_pid" 2>/dev/null || true
    wait "$vite_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cd "$repo_root"

if curl --silent --fail --output /dev/null http://127.0.0.1:1420/; then
  echo "Using the existing Vite server at http://127.0.0.1:1420/."
else
  vite_bin="$repo_root/node_modules/.bin/vite"
  if [[ ! -x "$vite_bin" ]]; then
    echo "Vite is not installed. Run 'npm ci' from the repository root first." >&2
    exit 1
  fi

  "$vite_bin" &
  vite_pid=$!

  for _ in {1..120}; do
    if curl --silent --fail --output /dev/null http://127.0.0.1:1420/; then
      break
    fi

    if ! kill -0 "$vite_pid" 2>/dev/null; then
      wait "$vite_pid"
      exit 1
    fi

    sleep 0.25
  done

  if ! curl --silent --fail --output /dev/null http://127.0.0.1:1420/; then
    echo "Vite did not become ready on port 1420 within 30 seconds." >&2
    exit 1
  fi
fi

cd "$repo_root/src-tauri"
cargo run "$@"
