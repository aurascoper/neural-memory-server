#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

[[ $# -eq 0 ]] || { echo "evidence MCP wrapper accepts no arguments" >&2; exit 2; }
/usr/bin/mountpoint -q /srv/neural-memory-data || { echo "encrypted data mount unavailable" >&2; exit 1; }
database=/srv/neural-memory-data/evidence/store.db
[[ -f $database && ! -L $database ]] || { echo "encrypted evidence database unavailable" >&2; exit 1; }
as_of=$(/usr/bin/date -u +%Y-%m-%dT%H:%M:%SZ)

# DELIBERATE: no --embed-url/--embed-profile, so the semantic branch is off in
# this deployment. Not an oversight — M3 replication measured semantic at
# +0.038 MRR against the +0.050 pre-registered bar
# (docs/acceptance/m3-retrieval-replication.md). Do not wire these flags in as
# a "fix". If semantic is ever wanted on, that is a new pre-registration with
# fresh data (world-model §4): a below-bar result gets a new registration,
# never a lowered bar.
exec /usr/bin/env -i \
    SQLITE_TMPDIR=/srv/neural-memory-data/sqlite-tmp \
    /usr/local/bin/neural-memory-mcp \
    --db /srv/neural-memory-data/evidence/store.db \
    --as-of "$as_of"
