#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

usage() {
    echo "usage: $0 --source ABSOLUTE_STORE_DB --admin ABSOLUTE_ADMIN_BINARY --writers-stopped [--apply]" >&2
    exit 2
}

source_db=
admin=
writers_stopped=false
apply=false
while (($#)); do
    case "$1" in
        --source) (($# >= 2)) || usage; source_db=$2; shift 2 ;;
        --admin) (($# >= 2)) || usage; admin=$2; shift 2 ;;
        --writers-stopped) writers_stopped=true; shift ;;
        --apply) apply=true; shift ;;
        *) usage ;;
    esac
done

destination=/srv/neural-memory-data/evidence/store.db
staging=/srv/neural-memory-data/staging/evidence-store.db
[[ $source_db == /* && $admin == /* ]] || usage
[[ $source_db != /srv/neural-memory-data/* && $source_db != / ]] || { echo "source must be a plaintext evidence store outside the encrypted mount" >&2; exit 2; }
$writers_stopped || { echo "--writers-stopped acknowledgement is required" >&2; exit 2; }
[[ -f $source_db ]] || { echo "source evidence database does not exist" >&2; exit 1; }
[[ -x $admin ]] || { echo "admin binary is not executable" >&2; exit 1; }

if ! $apply; then
    printf 'DRY-RUN verify mount: /srv/neural-memory-data\n'
    printf 'DRY-RUN VACUUM INTO and verify via: %s backup --db %s --to %s\n' "$admin" "$source_db" "$staging"
    printf 'DRY-RUN install verified evidence database: %s\n' "$destination"
    printf 'DRY-RUN retain staging as mode 0600 neural-memory:neural-memory: %s\n' "$staging"
    printf 'DRY-RUN retain plaintext original unchanged: %s\n' "$source_db"
    exit 0
fi

[[ $EUID -eq 0 ]] || { echo "--apply requires root" >&2; exit 1; }
mountpoint -q /srv/neural-memory-data || { echo "encrypted mount is unavailable" >&2; exit 1; }
if command -v fuser >/dev/null && fuser "$source_db" >/dev/null 2>&1; then
    echo "source database still has an open user; stop all writers" >&2
    exit 1
fi
[[ ! -e $staging && ! -e $destination ]] || { echo "staging or destination already exists; refusing overwrite" >&2; exit 1; }
install -d -m 0700 -o neural-memory -g neural-memory /srv/neural-memory-data/evidence /srv/neural-memory-data/staging
"$admin" backup --db "$source_db" --to "$staging"
"$admin" verify-backup --db "$source_db" --of "$staging"
chown neural-memory:neural-memory -- "$staging"
chmod 0600 -- "$staging"
[[ -f $staging && ! -L $staging ]] || { echo "verified staging copy is not a regular file" >&2; exit 1; }
[[ $(stat -c '%a:%U:%G' -- "$staging") == 600:neural-memory:neural-memory ]] || { echo "verified staging copy has unsafe metadata" >&2; exit 1; }
install -m 0600 -o neural-memory -g neural-memory "$staging" "$destination"
[[ -f $destination && ! -L $destination ]] || { echo "encrypted destination is not a regular file" >&2; exit 1; }
[[ $(stat -c '%a:%U:%G' -- "$destination") == 600:neural-memory:neural-memory ]] || { echo "encrypted destination has unsafe metadata" >&2; exit 1; }
"$admin" verify-backup --db "$source_db" --of "$destination"
echo "verified encrypted evidence copy created; plaintext original retained at $source_db" >&2
echo "verified staging copy retained at $staging until explicit acceptance" >&2
echo "switch the fixed evidence service path only after operator acceptance" >&2
