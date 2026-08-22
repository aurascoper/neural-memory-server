#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

if [[ -z ${NEURAL_MEMORY_PERSONAL_SYNC_BIN-} || -z ${NEURAL_MEMORY_PERSONAL_DB-} || -z ${NEURAL_MEMORY_PERSONAL_KEY-} || -z ${NEURAL_MEMORY_MAC_PUBLIC_KEY-} || -z ${NEURAL_MEMORY_PERSONAL_DEVICE-} ]]; then
    config=/etc/neural-memory/personal-sync.conf
    [[ -f $config && ! -L $config ]] || { echo "trusted sync config unavailable" >&2; exit 2; }
    [[ $(stat -c '%u:%G:%a' -- "$config") == 0:neural-memory:640 ]] || { echo "trusted sync config has unsafe ownership or mode" >&2; exit 2; }
    # shellcheck source=/dev/null
    source "$config"
fi

: "${NEURAL_MEMORY_PERSONAL_SYNC_BIN:?must name the fixed sync executable}"
: "${NEURAL_MEMORY_PERSONAL_DB:?must name the fixed personal.db path}"
: "${NEURAL_MEMORY_PERSONAL_KEY:?must name the fixed signing-key path}"
: "${NEURAL_MEMORY_MAC_PUBLIC_KEY:?must name the fixed enrolled Mac public-key path}"
: "${NEURAL_MEMORY_PERSONAL_DEVICE:?must name this device}"

command=${SSH_ORIGINAL_COMMAND-}
if [[ -z "$command" || "$command" =~ [\;\&\|\<\>\`\$\(\)\{\}\[\]\\\*\?\!\~] ]]; then
    echo "rejected sync command" >&2
    exit 2
fi
read -r -a argv <<< "$command"

if [[ ${#argv[@]} -eq 1 && ${argv[0]} == status ]]; then
    if /usr/bin/mountpoint -q /srv/neural-memory-data; then
        printf '%s\n' '{"health":"ready"}'
    else
        printf '%s\n' '{"health":"blocked-on-mount"}'
    fi
    exit 0
fi

if ! /usr/bin/mountpoint -q /srv/neural-memory-data; then
    echo "personal data mount unavailable" >&2
    exit 75
fi

case "${argv[*]}" in
    "public-key")
        exec /usr/bin/env -i \
            NEURAL_MEMORY_PERSONAL_DB="$NEURAL_MEMORY_PERSONAL_DB" \
            NEURAL_MEMORY_PERSONAL_KEY="$NEURAL_MEMORY_PERSONAL_KEY" \
            NEURAL_MEMORY_MAC_PUBLIC_KEY="$NEURAL_MEMORY_MAC_PUBLIC_KEY" \
            NEURAL_MEMORY_PERSONAL_DEVICE="$NEURAL_MEMORY_PERSONAL_DEVICE" \
            "$NEURAL_MEMORY_PERSONAL_SYNC_BIN" public-key
        ;;
esac

if [[ ${#argv[@]} -eq 3 && ${argv[0]} == export && ${argv[1]} == --after && ${argv[2]} =~ ^[0-9]+:[0-9]+$ ]]; then
    exec /usr/bin/env -i NEURAL_MEMORY_PERSONAL_DB="$NEURAL_MEMORY_PERSONAL_DB" NEURAL_MEMORY_PERSONAL_KEY="$NEURAL_MEMORY_PERSONAL_KEY" NEURAL_MEMORY_PERSONAL_DEVICE="$NEURAL_MEMORY_PERSONAL_DEVICE" "$NEURAL_MEMORY_PERSONAL_SYNC_BIN" export --after "${argv[2]}"
fi
if [[ ${#argv[@]} -eq 3 && ${argv[0]} == acknowledge && ${argv[1]} == --through && ${argv[2]} =~ ^[0-9]+:[0-9]+$ ]]; then
    exec /usr/bin/env -i NEURAL_MEMORY_PERSONAL_DB="$NEURAL_MEMORY_PERSONAL_DB" NEURAL_MEMORY_PERSONAL_KEY="$NEURAL_MEMORY_PERSONAL_KEY" NEURAL_MEMORY_PERSONAL_DEVICE="$NEURAL_MEMORY_PERSONAL_DEVICE" "$NEURAL_MEMORY_PERSONAL_SYNC_BIN" acknowledge --through "${argv[2]}"
fi
if [[ ${#argv[@]} -eq 1 && ${argv[0]} == import ]]; then
    exec /usr/bin/env -i NEURAL_MEMORY_PERSONAL_DB="$NEURAL_MEMORY_PERSONAL_DB" NEURAL_MEMORY_PERSONAL_KEY="$NEURAL_MEMORY_PERSONAL_KEY" NEURAL_MEMORY_MAC_PUBLIC_KEY="$NEURAL_MEMORY_MAC_PUBLIC_KEY" NEURAL_MEMORY_PERSONAL_DEVICE="$NEURAL_MEMORY_PERSONAL_DEVICE" "$NEURAL_MEMORY_PERSONAL_SYNC_BIN" import
fi

echo "rejected sync command" >&2
exit 2
