#!/usr/bin/env bash
set -euo pipefail
set -f

usage() {
    echo "usage: $0 [--root PATH] [--apply]" >&2
    exit 2
}

root=/
apply=false
while (($#)); do
    case "$1" in
        --root) (($# >= 2)) || usage; root=$2; shift 2 ;;
        --apply) apply=true; shift ;;
        *) usage ;;
    esac
done
[[ $root == /* ]] || usage
root=$(realpath -m -- "$root")
if $apply && [[ $root == / && $EUID -ne 0 ]]; then
    echo "live installation requires root" >&2
    exit 1
fi

script_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
crypt_entry=$(grep -E '^[^#[:space:]]' "$script_root/deploy/crypttab/neural-memory-data.conf")
fstab_entry=$(grep -E '^[^#[:space:]]' "$script_root/deploy/fstab/neural-memory-data.conf")
if [[ $root == / ]]; then
    etc=/etc
else
    etc=$root/etc
fi
crypttab=$etc/crypttab
fstab=$etc/fstab

for target in "$crypttab" "$fstab"; do
    [[ ! -L $target ]] || { echo "refusing symlink: $target" >&2; exit 1; }
done

check_crypttab() {
    local target=$1
    local status
    [[ -e $target ]] || return 0
    if awk -v expected="$crypt_entry" '
        /^[[:space:]]*(#|$)/ { next }
        $1 == "neural-memory-data" || $2 == "/var/lib/neural-memory/data.luks" {
            if ($0 != expected) { conflict = 1; exit 9 }
            found = 1
        }
        END { if (conflict) exit 9; if (found) exit 10 }
    ' "$target"; then
        return 0
    else
        status=$?
        case $status in
            9) echo "conflicting /etc/crypttab mapper or source entry" >&2; return 20 ;;
            10) return 10 ;;
            *) return "$status" ;;
        esac
    fi
}

check_fstab() {
    local target=$1
    local status
    [[ -e $target ]] || return 0
    if awk -v expected="$fstab_entry" '
        /^[[:space:]]*(#|$)/ { next }
        $1 == "/dev/mapper/neural-memory-data" || $2 == "/srv/neural-memory-data" {
            if ($0 != expected) { conflict = 1; exit 9 }
            found = 1
        }
        END { if (conflict) exit 9; if (found) exit 10 }
    ' "$target"; then
        return 0
    else
        status=$?
        case $status in
            9) echo "conflicting /etc/fstab source or mount entry" >&2; return 20 ;;
            10) return 10 ;;
            *) return "$status" ;;
        esac
    fi
}

crypt_present=false
fstab_present=false
if check_crypttab "$crypttab"; then
    :
else
    status=$?
    case $status in
        10) crypt_present=true ;;
        *) exit "$status" ;;
    esac
fi
if check_fstab "$fstab"; then
    :
else
    status=$?
    case $status in
        10) fstab_present=true ;;
        *) exit "$status" ;;
    esac
fi

if ! $apply; then
    $crypt_present || printf 'DRY-RUN append to %s: %s\n' "$crypttab" "$crypt_entry"
    $fstab_present || printf 'DRY-RUN append to %s: %s\n' "$fstab" "$fstab_entry"
    $crypt_present && $fstab_present && echo "DRY-RUN already installed"
    exit 0
fi

install -d -m 0755 -- "$etc"
preflight_backup() {
    local target=$1 present=$2
    if ! $present && [[ -e $target && -e $target.neural-memory-backup ]]; then
        echo "refusing to replace backup: $target.neural-memory-backup" >&2
        return 1
    fi
}
# Both destinations are checked before either source file or backup is changed.
preflight_backup "$crypttab" "$crypt_present"
preflight_backup "$fstab" "$fstab_present"
append_entry() {
    local target=$1 entry=$2 present=$3
    $present && return
    if [[ -e $target ]]; then
        backup=$target.neural-memory-backup
        cp -a -- "$target" "$backup"
    fi
    printf '%s\n' "$entry" >>"$target"
    chmod 0644 -- "$target"
}
append_entry "$crypttab" "$crypt_entry" "$crypt_present"
append_entry "$fstab" "$fstab_entry" "$fstab_present"
