#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

if [[ -z ${NEURAL_MEMORY_EVIDENCE_DR_BIN-} || -z ${NEURAL_MEMORY_EVIDENCE_DR_DIR-} ]]; then
    config=/etc/neural-memory/evidence-dr-pull.conf
    [[ -f $config && ! -L $config ]] || { echo "trusted DR config unavailable" >&2; exit 2; }
    [[ $(stat -c '%u:%G:%a' -- "$config") == 0:neural-memory:640 ]] || { echo "trusted DR config has unsafe ownership or mode" >&2; exit 2; }
    # shellcheck source=/dev/null
    source "$config"
fi

: "${NEURAL_MEMORY_EVIDENCE_DR_BIN:?fixed DR binary is required}"
: "${NEURAL_MEMORY_EVIDENCE_DR_DIR:?fixed DR directory is required}"

command=${SSH_ORIGINAL_COMMAND-}
if [[ -z $command || $command =~ [\;\&\|\<\>\`\$\(\)\{\}\[\]\\\*\?\!\~] ]]; then
    echo "rejected evidence DR command" >&2
    exit 2
fi
read -r -a argv <<<"$command"

if [[ ${#argv[@]} -eq 1 && ${argv[0]} == list ]]; then
    exec /usr/bin/env -i \
        NEURAL_MEMORY_EVIDENCE_DR_DIR="$NEURAL_MEMORY_EVIDENCE_DR_DIR" \
        "$NEURAL_MEMORY_EVIDENCE_DR_BIN" list
fi
if [[ ${#argv[@]} -eq 2 && ${argv[0]} == stream && ${argv[1]} =~ ^(backup|manifest|signature)$ ]]; then
    exec /usr/bin/env -i NEURAL_MEMORY_EVIDENCE_DR_DIR="$NEURAL_MEMORY_EVIDENCE_DR_DIR" "$NEURAL_MEMORY_EVIDENCE_DR_BIN" stream "${argv[1]}"
fi

echo "rejected evidence DR command" >&2
exit 2
