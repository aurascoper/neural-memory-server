#!/usr/bin/env bash
# Offline (no network), read-only health report for neural-memory-server.
# No destructive operations. Run by com.feynman.offlinedream.neural-memory-server.timer.
set -uo pipefail
cd "$(dirname "$0")/.."
OUT="notes/offline-dream-report.jsonl"
mkdir -p notes
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD_RC=1; TEST_RC=1
cargo build --workspace 2>/tmp/ndreamm-build.log; BUILD_RC=$?
cargo test --workspace 2>/tmp/ndreamm-test.log; TEST_RC=$?
GIT_LOG=$(git log --oneline -10 2>/dev/null | sed 's/"/\\"/g')
python3 - "$TS" "$BUILD_RC" "$TEST_RC" "$GIT_LOG" >> "$OUT" <<'PYEOF'
import json, sys
ts, build_rc, test_rc, git_log = sys.argv[1:5]
print(json.dumps({
    "timestamp": ts, "script": "offline-dream-report.sh",
    "network_calls_made": False,
    "build_rc": int(build_rc), "test_rc": int(test_rc),
    "recent_commits": git_log.splitlines(),
}))
PYEOF
