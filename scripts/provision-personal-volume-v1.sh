#!/usr/bin/env bash
set -euo pipefail
set -f

usage() {
    echo "usage: $0 --image /var/lib/neural-memory/data.luks --recovery-key-file PATH [--apply]" >&2
    exit 2
}

apply=false
image=
recovery_key=
while (($#)); do
    case "$1" in
        --image) (($# >= 2)) || usage; image=$2; shift 2 ;;
        --recovery-key-file) (($# >= 2)) || usage; recovery_key=$2; shift 2 ;;
        --apply) apply=true; shift ;;
        *) usage ;;
    esac
done

[[ $image == /var/lib/neural-memory/data.luks && -n $recovery_key && $recovery_key == /* ]] || usage
[[ -f $recovery_key ]] || { echo "recovery key does not exist" >&2; exit 2; }
key_mode=$(stat -c '%a' -- "$recovery_key")
[[ $key_mode == 600 || $key_mode == 400 ]] || { echo "recovery key must have mode 0600 or 0400" >&2; exit 2; }

mountpoint=/srv/neural-memory-data
mapper=neural-memory-data
layout=(personal evidence backups staging sqlite-tmp outboxes keys logs)

if ! $apply; then
    printf 'DRY-RUN create sparse LUKS2 volume: %s (16 GiB)\n' "$image"
    printf 'DRY-RUN open mapper: /dev/mapper/%s\n' "$mapper"
    printf 'DRY-RUN mount with allow-discards at: %s\n' "$mountpoint"
    printf 'DRY-RUN create mode 0700 layout:'
    printf ' %s' "${layout[@]}"
    printf '\nDRY-RUN recovery key remains external; enroll TPM2 only after recovery unlock succeeds\n'
    exit 0
fi

[[ $EUID -eq 0 ]] || { echo "--apply requires root" >&2; exit 1; }
for command in truncate losetup cryptsetup mkfs.ext4 mount mountpoint findmnt install blkid grep sed stat getent realpath; do
    command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 1; }
done
getent passwd neural-memory >/dev/null || { echo "missing service account: neural-memory" >&2; exit 1; }

if [[ ! -e $image ]]; then
    install -d -m 0700 -- "$(dirname -- "$image")"
    truncate -s 16G -- "$image"
    chmod 0600 -- "$image"
fi
[[ $(stat -c '%s' -- "$image") == 17179869184 ]] || { echo "image must be exactly 16 GiB" >&2; exit 1; }
[[ $(stat -c '%a' -- "$image") == 600 ]] || { echo "image must have mode 0600" >&2; exit 1; }
loop=$(losetup -j "$image" | sed -n '1s/:.*//p')
if [[ -z $loop ]]; then
    loop=$(losetup --find --show --direct-io=on "$image")
fi
if ! cryptsetup isLuks "$loop" >/dev/null 2>&1; then
    cryptsetup luksFormat --type luks2 --batch-mode --key-file "$recovery_key" "$loop"
fi
if [[ ! -e /dev/mapper/$mapper ]]; then
    cryptsetup open --allow-discards --key-file "$recovery_key" "$loop" "$mapper"
fi
mapped_device=$(cryptsetup status "$mapper" | sed -n 's/^[[:space:]]*device:[[:space:]]*//p')
[[ -n $mapped_device && $(realpath -- "$mapped_device") == $(realpath -- "$loop") ]] || { echo "mapper is attached to a different backing device" >&2; exit 1; }
if ! blkid -o value -s TYPE "/dev/mapper/$mapper" | grep -qx ext4; then
    mkfs.ext4 -L neural-memory-data "/dev/mapper/$mapper"
fi
install -d -m 0700 -- "$mountpoint"
if ! mountpoint -q "$mountpoint"; then
    mount -o nosuid,nodev,noexec "/dev/mapper/$mapper" "$mountpoint"
fi
findmnt -rn --source "/dev/mapper/$mapper" --target "$mountpoint" >/dev/null || { echo "mountpoint has an unexpected source" >&2; exit 1; }
chown root:neural-memory -- "$mountpoint"
chmod 0750 -- "$mountpoint"
for directory in "${layout[@]}"; do
    install -d -m 0700 -o neural-memory -g neural-memory -- "$mountpoint/$directory"
done
findmnt -rn -o SOURCE,OPTIONS --target "$mountpoint"
cryptsetup status "$mapper"
