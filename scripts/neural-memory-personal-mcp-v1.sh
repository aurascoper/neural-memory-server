#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

[[ $# -eq 0 ]] || { echo "personal MCP wrapper accepts no arguments" >&2; exit 2; }
/usr/bin/mountpoint -q /srv/neural-memory-data || { echo "encrypted data mount unavailable" >&2; exit 1; }

exec /usr/bin/env -i \
    SQLITE_TMPDIR=/srv/neural-memory-data/sqlite-tmp \
    /usr/local/bin/neural-memory-personal-mcp \
    --db /srv/neural-memory-data/personal/personal.db
