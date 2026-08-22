#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

[[ $# -eq 0 ]] || { echo "embedding worker accepts no arguments" >&2; exit 2; }
/usr/bin/mountpoint -q /srv/neural-memory-data || { echo "encrypted data mount unavailable" >&2; exit 1; }
now=$(/usr/bin/date -u +%Y-%m-%dT%H:%M:%S.%NZ)
at=${now:0:23}Z

profile=(
    --backend llama.cpp-cpu
    --model-artifact sha256:3e24342164b3d94991ba9692fdc0dd08e3fd7362e0aacc396a9a5c54a544c3b7
    --dimension 768
    --normalization l2
    --version d0bfb1981266c271cd0536a8aa7c5e863e7cdf61
    --adapter llama-cpp-http
    --endpoint http://127.0.0.1:8082
)
/usr/local/bin/neural-memory-personal-admin profile-set \
    --db /srv/neural-memory-data/personal/personal.db \
    "${profile[@]}" --at "$at"
/usr/local/bin/neural-memory-personal-admin rebuild \
    --db /srv/neural-memory-data/personal/personal.db \
    "${profile[@]}" --limit 100 --at "$at"
