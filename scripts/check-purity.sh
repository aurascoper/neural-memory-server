#!/usr/bin/env bash
# The effect-free doctrine, enforced rather than intended.
#
# neural-memory-domain must not be able to touch a database, an async runtime, a
# clock, or a UUID. Stating that in a doc comment is not enforcement -- a future
# `cargo add` would silently break it. This asserts it against the resolved
# dependency tree.
#
# The UUID ban matters most. Primary keys are foreign-key targets; the 64-hex
# seal is the identity. If the pure crate could mint a UUID, the two could be
# confused with nothing failing. (The schema goes further and contains no UUIDs
# at all -- a record's key IS its digest.)
set -euo pipefail
cd "$(dirname "$0")/.."

BANNED=(rusqlite tokio uuid libsqlite3-sys async-std reqwest chrono time)
FAILED=0

TREE="$(cargo tree -p neural-memory-domain --edges normal --prefix none 2>/dev/null)"

echo "neural-memory-domain resolved dependencies:"
echo "$TREE" | sed 's/^/  /'
echo

for crate in "${BANNED[@]}"; do
  if echo "$TREE" | grep -qE "^${crate} v"; then
    echo "FAIL: '${crate}' is in neural-memory-domain's dependency tree"
    FAILED=1
  fi
done

# A clock read is the specific effect the doctrine names, so look for it in the
# source too -- a std call needs no dependency to give it away.
if grep -rnE 'SystemTime::now|Instant::now|std::process|std::fs|std::net' \
     crates/neural-memory-domain/src/ 2>/dev/null; then
  echo "FAIL: neural-memory-domain reads a clock, the filesystem, or the network"
  FAILED=1
fi

if [ "$FAILED" -eq 0 ]; then
  echo "PASS: domain crate is pure (no db, no async runtime, no clock, no uuid)"
fi
exit "$FAILED"
