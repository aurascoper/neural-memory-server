#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

[[ $# -eq 0 ]] || { echo "embedding model verifier accepts no arguments" >&2; exit 2; }
model=/usr/local/share/neural-memory/models/nomic-embed-text-v1.5.Q8_0.gguf
expected_sha256=3e24342164b3d94991ba9692fdc0dd08e3fd7362e0aacc396a9a5c54a544c3b7
expected_bytes=146146432

[[ -f $model && ! -L $model ]] || { echo "fixed embedding model unavailable" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%s' -- "$model") == "$expected_bytes" ]] || { echo "embedding model byte length mismatch" >&2; exit 1; }
actual_sha256=$(/usr/bin/sha256sum -- "$model")
[[ ${actual_sha256%% *} == "$expected_sha256" ]] || { echo "embedding model SHA-256 mismatch" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%U:%G:%a' -- "$model") == root:neural-memory:640 ]] || { echo "embedding model metadata is unsafe" >&2; exit 1; }
