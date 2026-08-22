#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

[[ $# -eq 0 ]] || {
    echo "usage: neural-memory-wait-embedding-ready-v1" >&2
    exit 2
}

endpoint=http://127.0.0.1:8082/v1/embeddings
timeout_seconds=90
started=$SECONDS
probe=readiness
for ((word = 0; word < 1800; word++)); do
    probe+=' readiness'
done
payload=$(printf '{"input":"%s","model":"embed"}' "$probe")

while (( SECONDS - started < timeout_seconds )); do
    if /usr/bin/curl --silent --show-error --fail \
        --connect-timeout 1 --max-time 2 \
        --header 'Content-Type: application/json' \
        --data "$payload" \
        --output /dev/null -- "$endpoint"; then
        exit 0
    fi
    /usr/bin/sleep 1
done

echo "embedding endpoint did not become ready within ${timeout_seconds}s" >&2
exit 1
