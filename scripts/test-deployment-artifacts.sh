#!/usr/bin/env bash
set -euo pipefail
set -f

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }

for script in \
    "$root/scripts/provision-personal-volume-v1.sh" \
    "$root/scripts/migrate-evidence-to-encrypted-v1.sh" \
    "$root/scripts/evidence-dr-forced-command-v1.sh" \
    "$root/scripts/personal-sync-forced-command-v1.sh" \
    "$root/scripts/install-neural-memory-mount-entries-v1.sh" \
    "$root/scripts/neural-memory-personal-mcp-v1.sh" \
    "$root/scripts/neural-memory-evidence-mcp-v1.sh" \
    "$root/scripts/neural-memory-verify-llama-runtime-v1.sh" \
    "$root/scripts/neural-memory-verify-embedding-model-v1.sh" \
    "$root/scripts/neural-memory-wait-embedding-ready-v1.sh" \
    "$root/scripts/neural-memory-embedding-worker-v1.sh"; do
    bash -n "$script" || fail "syntax: $script"
done

while IFS= read -r unit; do
    grep -q '^NoNewPrivileges=true$' "$unit" || fail "privilege boundary: $unit"
    grep -q '^UMask=0077$' "$unit" || fail "private creation mask: $unit"
    if [[ $(basename "$unit") != neural-memory-embedding-server.service ]]; then
        grep -q '^RequiresMountsFor=/srv/neural-memory-data$' "$unit" || fail "mount dependency: $unit"
        grep -q '^ConditionPathIsMountPoint=/srv/neural-memory-data$' "$unit" || fail "mount-point condition: $unit"
        grep -q '/srv/neural-memory-data' "$unit" || fail "fixed encrypted path: $unit"
    fi
done < <(find "$root/deploy/systemd" -maxdepth 1 -type f -name '*.service' -print)

embedding_server=$root/deploy/systemd/neural-memory-embedding-server.service
embedding_worker=$root/deploy/systemd/neural-memory-embedding-worker.service
embedding_timer=$root/deploy/systemd/neural-memory-embedding-worker.timer
model_verifier=$root/scripts/neural-memory-verify-embedding-model-v1.sh
runtime_verifier=$root/scripts/neural-memory-verify-llama-runtime-v1.sh
readiness_helper=$root/scripts/neural-memory-wait-embedding-ready-v1.sh
worker_wrapper=$root/scripts/neural-memory-embedding-worker-v1.sh
runtime_manifest=$root/deploy/embedding/llama-cpu-runtime-v1.manifest
provenance_manifest=$root/deploy/embedding/embedding-provenance-v1.json
runtime_docs=$root/deploy/embedding/README.md
grep -Fq 'Environment=LD_LIBRARY_PATH=/usr/local/lib/neural-memory/llama-cpu' "$embedding_server" || fail "fixed llama runtime library path"
grep -Fq 'ExecStartPre=/usr/local/libexec/neural-memory-verify-llama-runtime-v1' "$embedding_server" || fail "runtime verification before server startup"
grep -Fq 'ExecStartPre=/usr/local/libexec/neural-memory-verify-embedding-model-v1' "$embedding_server" || fail "model verification before server startup"
grep -Fq 'ExecStartPost=/usr/local/libexec/neural-memory-wait-embedding-ready-v1' "$embedding_server" || fail "bounded embedding readiness gate"
grep -q '^ProtectHome=true$' "$embedding_server" || fail "embedding server home protection"
grep -Fq -- '--host 127.0.0.1 --port 8082 --embeddings --pooling mean --ctx-size 2048 --batch-size 2048 --ubatch-size 2048 --parallel 1 --n-gpu-layers 0' "$embedding_server" || fail "serial loopback CPU embedding server matches artifact context"
grep -q '^IPAddressDeny=any$' "$embedding_server" || fail "embedding server network deny"
grep -q '^IPAddressAllow=localhost$' "$embedding_server" || fail "embedding server loopback allow"
grep -q '^Requires=neural-memory-embedding-server.service$' "$embedding_worker" || fail "embedding worker server dependency"
grep -q '^RequiresMountsFor=/srv/neural-memory-data$' "$embedding_worker" || fail "embedding worker mount dependency"
grep -q '^ConditionPathIsMountPoint=/srv/neural-memory-data$' "$embedding_worker" || fail "embedding worker mount condition"
grep -q '^OnUnitActiveSec=1m$' "$embedding_timer" || fail "embedding worker timer interval"
grep -Fq 'expected_sha256=3e24342164b3d94991ba9692fdc0dd08e3fd7362e0aacc396a9a5c54a544c3b7' "$model_verifier" || fail "measured model SHA-256"
grep -Fq 'expected_bytes=146146432' "$model_verifier" || fail "measured model byte length"
grep -Fq 'expected_server_sha256=bd95aacd01abd53a2eed1a08c3e37f808d1760e7161ca7f5520a452ff81c0fe2' "$runtime_verifier" || fail "measured llama-server SHA-256"
grep -Fq 'expected_server_bytes=17904' "$runtime_verifier" || fail "measured llama-server byte length"
grep -Fq 'expected_short_commit=d0bfb1981' "$runtime_verifier" || fail "measured llama-server short commit"
grep -Fq 'expected_version=10188' "$runtime_verifier" || fail "measured llama-server version"
grep -Fq 'expected_provenance_sha256=f68c00856e107909fc593c8b448a4e4f26118d068744cceddc07fc2e0e5a9366' "$runtime_verifier" || fail "sealed embedding provenance"
grep -Fq 'provenance=/usr/local/share/neural-memory/embedding-provenance-v1.json' "$runtime_verifier" || fail "fixed embedding provenance path"
grep -Fq '"sourceCommit": "d0bfb1981266c271cd0536a8aa7c5e863e7cdf61"' "$provenance_manifest" || fail "full llama source commit provenance"
grep -Fq '"upstreamRepository": "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF"' "$provenance_manifest" || fail "model source repository provenance"
grep -Fq '"upstreamRevision": "18d1044f4866e224159fce8c6fc5c4f3920176e7"' "$provenance_manifest" || fail "model source revision provenance"
! grep -Fq '"upstreamRevision": "unknown"' "$provenance_manifest" || fail "unknown model source revision"
grep -Fq '"artifactDeclaredTrainingContext": 2048' "$provenance_manifest" || fail "artifact context provenance"
[[ $(wc -l <"$runtime_manifest") -eq 7 ]] || fail "llama runtime manifest entry count"
grep -Fq 'libllama-server-impl.so 7682544 796c1617dc46bdb83b0721a13158c58e45e578abb7abb9f124ce346c05a8f9b1' "$runtime_manifest" || fail "server implementation runtime seal"
! grep -R -q '/home/' "$embedding_server" "$runtime_verifier" "$model_verifier" "$readiness_helper" "$runtime_manifest" "$provenance_manifest" "$runtime_docs" || fail "embedding runtime depends on /home"
grep -Fq 'endpoint=http://127.0.0.1:8082/v1/embeddings' "$readiness_helper" || fail "fixed embedding readiness endpoint"
grep -Fq 'timeout_seconds=90' "$readiness_helper" || fail "bounded embedding readiness timeout"
grep -Fq 'word < 1800' "$readiness_helper" || fail "readiness probe is in artifact context and exceeds former batch ceiling"
grep -Fq -- '--data "$payload"' "$readiness_helper" || fail "actual embedding readiness request"
if "$readiness_helper" unexpected >/dev/null 2>&1; then
    fail "embedding readiness helper accepted caller arguments"
fi
grep -Fq -- '--limit 100' "$worker_wrapper" || fail "bounded embedding batch"
grep -Fq 'now=$(/usr/bin/date -u +%Y-%m-%dT%H:%M:%S.%NZ)' "$worker_wrapper" || fail "worker full timestamp source"
grep -Fq 'at=${now:0:23}Z' "$worker_wrapper" || fail "worker millisecond timestamp truncation"
grep -Fq -- '--endpoint http://127.0.0.1:8082' "$worker_wrapper" || fail "strict worker endpoint"
! grep -Eiq '(semantic|ranking|store\.db|/home/)' "$worker_wrapper" || fail "embedding worker crosses scope"
if "$worker_wrapper" unexpected >/dev/null 2>&1; then
    fail "embedding worker accepted caller arguments"
fi
if "$model_verifier" unexpected >/dev/null 2>&1; then
    fail "model verifier accepted caller arguments"
fi

for wrapper in \
    "$root/scripts/neural-memory-personal-mcp-v1.sh" \
    "$root/scripts/neural-memory-evidence-mcp-v1.sh" \
    "$root/scripts/personal-sync-forced-command-v1.sh" \
    "$root/scripts/evidence-dr-forced-command-v1.sh"; do
    grep -q '^umask 077$' "$wrapper" || fail "private creation mask: $wrapper"
done

[[ ! -e $root/deploy/systemd/neural-memory-personal-mcp.service ]] || fail "personal stdio MCP is a daemon"
[[ ! -e $root/deploy/systemd/neural-memory-evidence.service ]] || fail "evidence stdio MCP is a daemon"
grep -q '^ExecStart=.*neural-memory-personal-transport' "$root/deploy/systemd/neural-memory-personal-transport.service" || fail "transport system service retained"

local_sudoers=$root/deploy/sudoers/neural-memory-local-mcp
! grep -Eq '^[[:space:]]*Defaults' "$local_sudoers" || fail "local MCP sudoers changes unrelated defaults"
grep -Fq 'aurascoper ALL=(neural-memory) NOPASSWD: /usr/local/libexec/neural-memory-personal-mcp-v1 ""' "$local_sudoers" || fail "personal MCP exact sudo rule"
grep -Fq 'aurascoper ALL=(neural-memory) NOPASSWD: /usr/local/libexec/neural-memory-evidence-mcp-v1 ""' "$local_sudoers" || fail "evidence MCP exact sudo rule"
[[ $(grep -c 'NOPASSWD:' "$local_sudoers") -eq 2 ]] || fail "unexpected local MCP sudo rule"
! grep -Eiq '(ALL[[:space:]]*\)|/bin/(ba)?sh|sudoedit|SETENV|\*)' "$local_sudoers" || fail "local MCP unrestricted sudo surface"
if command -v visudo >/dev/null; then
    visudo -cf "$local_sudoers" >/dev/null || fail "local MCP sudoers syntax"
else
    echo "SKIP: visudo unavailable" >&2
fi

personal_wrapper=$root/scripts/neural-memory-personal-mcp-v1.sh
evidence_wrapper=$root/scripts/neural-memory-evidence-mcp-v1.sh
grep -Fq '[[ $# -eq 0 ]]' "$personal_wrapper" || fail "personal MCP no-argument grammar"
grep -Fq '/usr/bin/mountpoint -q /srv/neural-memory-data' "$personal_wrapper" || fail "personal MCP fixed mountpoint helper"
grep -Fq 'exec /usr/bin/env -i' "$personal_wrapper" || fail "personal MCP fixed env helper"
grep -Fq -- '--db /srv/neural-memory-data/personal/personal.db' "$personal_wrapper" || fail "personal MCP fixed database"
! grep -Eiq '(store\.db|--as-of|SSH_ORIGINAL_COMMAND|(^|[^A-Za-z])(recall|sql)([^A-Za-z]|$))' "$personal_wrapper" || fail "personal MCP forbidden surface"
grep -Fq '[[ $# -eq 0 ]]' "$evidence_wrapper" || fail "evidence MCP no-argument grammar"
grep -Fq '/usr/bin/mountpoint -q /srv/neural-memory-data' "$evidence_wrapper" || fail "evidence MCP fixed mountpoint helper"
grep -Fq 'as_of=$(/usr/bin/date -u +%Y-%m-%dT%H:%M:%SZ)' "$evidence_wrapper" || fail "evidence MCP fixed date helper"
grep -Fq 'exec /usr/bin/env -i' "$evidence_wrapper" || fail "evidence MCP fixed env helper"
grep -Fq -- '--db /srv/neural-memory-data/evidence/store.db' "$evidence_wrapper" || fail "evidence MCP fixed database"
grep -Fq -- '--as-of "$as_of"' "$evidence_wrapper" || fail "evidence MCP generated as-of"
! grep -Eiq '(personal\.db|SSH_ORIGINAL_COMMAND|(^|[^A-Za-z])(recall|sql)([^A-Za-z]|$))' "$evidence_wrapper" || fail "evidence MCP forbidden surface"
if "$personal_wrapper" unexpected >/dev/null 2>&1; then
    fail "personal MCP wrapper accepted caller arguments"
fi
if "$evidence_wrapper" unexpected >/dev/null 2>&1; then
    fail "evidence MCP wrapper accepted caller arguments"
fi

if grep -q 'personal\.db' "$root/scripts/migrate-evidence-to-encrypted-v1.sh"; then
    fail "evidence migration crosses the personal database boundary"
fi
grep -q 'destination=/srv/neural-memory-data/evidence/store.db' "$root/scripts/migrate-evidence-to-encrypted-v1.sh" || fail "fixed evidence destination"
migration_script=$root/scripts/migrate-evidence-to-encrypted-v1.sh
grep -q '^umask 077$' "$migration_script" || fail "evidence migration private creation mask"
grep -Fq 'chown neural-memory:neural-memory -- "$staging"' "$migration_script" || fail "retained staging ownership"
grep -Fq 'chmod 0600 -- "$staging"' "$migration_script" || fail "retained staging mode"
grep -Fq '600:neural-memory:neural-memory' "$migration_script" || fail "retained staging metadata assertion"
backup_line=$(grep -n '"$admin" backup --db' "$migration_script" | cut -d: -f1)
staging_verify_line=$(grep -n '"$admin" verify-backup --db "$source_db" --of "$staging"' "$migration_script" | cut -d: -f1)
staging_mode_line=$(grep -n 'chmod 0600 -- "$staging"' "$migration_script" | cut -d: -f1)
destination_line=$(grep -n 'install -m 0600 -o neural-memory' "$migration_script" | cut -d: -f1)
[[ $backup_line -lt $staging_verify_line && $staging_verify_line -lt $staging_mode_line && $staging_mode_line -lt $destination_line ]] || fail "evidence verifier/metadata/install order"
grep -Fq 'immutable=1' "$root/docs/gpd-deployment.md" || fail "immutable plaintext SQLite inspection"
grep -Fq 'Only when no `store.db-wal` exists' "$root/docs/gpd-deployment.md" || fail "immutable no-WAL limitation"
grep -Fq 'sha256sum /plaintext/store.db > store.db.before.sha256' "$root/docs/gpd-deployment.md" || fail "independent main-file hash"
grep -Fq 'sha256sum /plaintext/store.db-wal > store.db-wal.before.sha256' "$root/docs/gpd-deployment.md" || fail "independent WAL hash"
grep -Fq 'neural-memory-admin backup' "$root/docs/gpd-deployment.md" || fail "authoritative coherent evidence check"
grep -Fq 'Retain the plaintext main file and every' "$root/docs/gpd-deployment.md" || fail "plaintext sidecar retention"
grep -q 'mode 0600 or 0400' "$root/scripts/provision-personal-volume-v1.sh" || fail "recovery-key mode check"
grep -q 'install -d -m 0700' "$root/scripts/provision-personal-volume-v1.sh" || fail "directory mode"
grep -q 'chmod 0600 -- "$image"' "$root/scripts/provision-personal-volume-v1.sh" || fail "image mode"
grep -q '\[\[ \$image == /var/lib/neural-memory/data.luks' "$root/scripts/provision-personal-volume-v1.sh" || fail "fixed backing image"
! grep -E -q '^[[:space:]]*mount -o [^#]*discard' "$root/scripts/provision-personal-volume-v1.sh" || fail "continuous filesystem discard"
grep -q '^zram-size = 12288$' "$root/deploy/systemd/zram-generator.conf" || fail "12 GiB zram size in generator megabytes"
! grep -q '^zram-size = 12288M$' "$root/deploy/systemd/zram-generator.conf" || fail "zram size uses invalid multiplier suffix"
grep -q '^systemctl mask hibernate.target hybrid-sleep.target suspend-then-hibernate.target$' "$root/docs/gpd-deployment.md" || fail "hibernation-only mask set"
! grep -Eq 'systemctl mask.*[[:space:]](sleep|suspend)\.target([[:space:]]|$)' "$root/docs/gpd-deployment.md" || fail "ordinary suspend target is masked"
grep -q 'location = /v1/personal-sync' "$root/deploy/https/personal-sync-nginx.conf.example" || fail "exact HTTPS sync endpoint"
! grep -q -E 'recall|personal-mcp|sql' "$root/deploy/https/personal-sync-nginx.conf.example" || fail "HTTPS exposes a forbidden surface"
grep -q 'include /srv/neural-memory-data/keys/nginx-personal-sync-bearer.conf;' "$root/deploy/https/personal-sync-nginx.conf.example" || fail "nginx backend credential include"
grep -q -- '--listen 127.0.0.1:9443' "$root/deploy/systemd/neural-memory-personal-transport.service" || fail "transport loopback binding"
grep -q -- '--token-file /srv/neural-memory-data/keys/personal-transport.token' "$root/deploy/systemd/neural-memory-personal-transport.service" || fail "transport token path"
grep -Fq '{"action":"status"}' "$root/docs/gpd-deployment.md" || fail "HTTPS data-free status schema"
grep -Fq '`blocked-on-mount` remains' "$root/docs/gpd-deployment.md" || fail "SSH blocked-mount status distinction"

crypttab="$root/deploy/crypttab/neural-memory-data.conf"
fstab="$root/deploy/fstab/neural-memory-data.conf"
grep -q '^neural-memory-data /var/lib/neural-memory/data.luks none tpm2-device=auto,discard,luks$' "$crypttab" || fail "stable crypttab backing path"
! grep -q '/dev/loop' "$crypttab" || fail "crypttab persists a loop device"
grep -q '^/dev/mapper/neural-memory-data /srv/neural-memory-data ext4 nosuid,nodev,noexec 0 2$' "$fstab" || fail "hardened encrypted mount"
grep -q 'Append this entry to /etc/crypttab' "$crypttab" || fail "crypttab append semantics"
! grep -q 'crypttab\.d' "$crypttab" || fail "ignored crypttab drop-in claim"

check_transport_policy() {
    account=$1
    wrapper=$2
    sudoers=$3
    authorized_keys=$4
    grep -Fq "Defaults:$account env_reset" "$sudoers" || fail "$account env reset"
    grep -Fq "Defaults!$wrapper env_keep += \"SSH_ORIGINAL_COMMAND\"" "$sudoers" || fail "$account command-only SSH_ORIGINAL_COMMAND"
    grep -Fq "$account ALL=(neural-memory) NOPASSWD: $wrapper \"\"" "$sudoers" || fail "$account exact sudo rule"
    [[ $(grep -c 'NOPASSWD:' "$sudoers") -eq 1 ]] || fail "$account has multiple sudo rules"
    ! grep -Eiq '(ALL[[:space:]]*\)|/bin/(ba)?sh|sudoedit|SETENV|\*)' "$sudoers" || fail "$account unrestricted sudo surface"
    grep -Fq "command=\"/usr/bin/sudo -n -u neural-memory $wrapper\"" "$authorized_keys" || fail "$account forced sudo command"
    grep -q '^restrict,from=' "$authorized_keys" || fail "$account SSH restrictions"
    ! grep -Eiq '(recall|sql|personal\.db|store\.db|/bin/(ba)?sh)' "$authorized_keys" || fail "$account forbidden SSH surface"
}

check_transport_policy neural-memory-sync /usr/local/libexec/personal-sync-forced-command-v1 \
    "$root/deploy/sudoers/neural-memory-sync" "$root/deploy/ssh/authorized_keys.personal-sync.example"
check_transport_policy neural-memory-dr /usr/local/libexec/evidence-dr-forced-command-v1 \
    "$root/deploy/sudoers/neural-memory-dr" "$root/deploy/ssh/authorized_keys.evidence-dr.example"

grep -Fq 'config=/etc/neural-memory/personal-sync.conf' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "fixed sync config"
grep -Fq 'NEURAL_MEMORY_MAC_PUBLIC_KEY=/srv/neural-memory-data/keys/mac-ed25519.pub' "$root/config/personal-sync.conf.example" || fail "fixed enrolled Mac key config"
grep -Fq 'NEURAL_MEMORY_MAC_PUBLIC_KEY="$NEURAL_MEMORY_MAC_PUBLIC_KEY"' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "sync enrolled key propagation"
grep -Fq '${#argv[@]} -eq 1 && ${argv[0]} == import' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "bare sync import grammar"
! grep -Fq -- '--trusted-key-base64' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "request-selected sync trust"
grep -Fq -- '--peer-key /srv/neural-memory-data/keys/mac-ed25519.pub' "$root/deploy/systemd/neural-memory-personal-transport.service" || fail "transport enrolled peer key"
grep -Fq '0:neural-memory:640' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "sync config ownership"
grep -Fq '/usr/bin/mountpoint -q /srv/neural-memory-data' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "sync fixed mount preflight"
grep -Fq "'{\"health\":\"blocked-on-mount\"}'" "$root/scripts/personal-sync-forced-command-v1.sh" || fail "sync blocked mount JSON"
! grep -Eiq '(recall|sql|filesystem|store\.db)' "$root/scripts/personal-sync-forced-command-v1.sh" || fail "sync status expands forbidden surface"
grep -Fq 'config=/etc/neural-memory/evidence-dr-pull.conf' "$root/scripts/evidence-dr-forced-command-v1.sh" || fail "fixed DR config"
grep -Fq '0:neural-memory:640' "$root/scripts/evidence-dr-forced-command-v1.sh" || fail "DR config ownership"

temporary=$(mktemp -d)
cleanup() {
    rm -rf -- "$temporary"
}
trap cleanup EXIT
key=$temporary/recovery.key
: >"$key"
chmod 0600 "$key"
test_uid=$(id -u)
test_user=$(id -un)
test_group=$(id -gn)

timestamp_now=$(/usr/bin/date -u +%Y-%m-%dT%H:%M:%S.%NZ)
timestamp_at=${timestamp_now:0:23}Z
[[ $timestamp_at =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z$ ]] || fail "worker timestamp is not canonical milliseconds"

readiness_count=$temporary/readiness-count
readiness_curl=$temporary/readiness-curl
cat >"$readiness_curl" <<'EOF'
#!/usr/bin/env bash
count=0
[[ ! -f __COUNT__ ]] || read -r count <__COUNT__
count=$((count + 1))
printf '%s\n' "$count" >__COUNT__
(( count >= 3 ))
EOF
sed -i "s|__COUNT__|$readiness_count|g" "$readiness_curl"
chmod 0700 "$readiness_curl"
delayed_readiness=$temporary/delayed-readiness
sed -e "s|/usr/bin/curl|$readiness_curl|" \
    -e 's|/usr/bin/sleep 1|/usr/bin/sleep 0.01|' \
    "$readiness_helper" >"$delayed_readiness"
chmod 0700 "$delayed_readiness"
"$delayed_readiness" || fail "embedding readiness rejected delayed-ready endpoint"
[[ $(<"$readiness_count") -eq 3 ]] || fail "embedding readiness did not wait for success"

timeout_curl=$temporary/timeout-curl
printf '#!/usr/bin/env bash\nexit 22\n' >"$timeout_curl"
chmod 0700 "$timeout_curl"
timeout_readiness=$temporary/timeout-readiness
sed -e "s|/usr/bin/curl|$timeout_curl|" \
    -e 's|timeout_seconds=90|timeout_seconds=1|' \
    -e 's|/usr/bin/sleep 1|/usr/bin/sleep 0.01|' \
    "$readiness_helper" >"$timeout_readiness"
chmod 0700 "$timeout_readiness"
if "$timeout_readiness" >"$temporary/readiness-timeout.out" 2>"$temporary/readiness-timeout.err"; then
    fail "embedding readiness accepted persistent failure"
fi
grep -q 'did not become ready within 1s' "$temporary/readiness-timeout.err" || fail "embedding readiness timeout diagnostic"

missing_mount_worker=$temporary/embedding-worker-missing-mount
sed "s|/srv/neural-memory-data|$temporary/not-mounted|g" "$worker_wrapper" >"$missing_mount_worker"
chmod 0700 "$missing_mount_worker"
if "$missing_mount_worker" >"$temporary/worker.out" 2>"$temporary/worker.err"; then
    fail "embedding worker accepted missing encrypted mount"
fi
grep -q 'encrypted data mount unavailable' "$temporary/worker.err" || fail "embedding worker mount failure diagnostic"

missing_verifier=$temporary/verify-missing-model
sed "s|^model=.*|model=$temporary/missing-model.gguf|" "$model_verifier" >"$missing_verifier"
chmod 0700 "$missing_verifier"
if "$missing_verifier" >"$temporary/missing.out" 2>"$temporary/missing.err"; then
    fail "model verifier accepted unavailable model"
fi
grep -q 'fixed embedding model unavailable' "$temporary/missing.err" || fail "model unavailable diagnostic"

mismatch_model=$temporary/mismatch-model.gguf
truncate -s 146146432 "$mismatch_model"
mismatch_verifier=$temporary/verify-mismatch-model
sed "s|^model=.*|model=$mismatch_model|" "$model_verifier" >"$mismatch_verifier"
chmod 0700 "$mismatch_verifier"
if "$mismatch_verifier" >"$temporary/mismatch.out" 2>"$temporary/mismatch.err"; then
    fail "model verifier accepted SHA-256 mismatch"
fi
grep -q 'embedding model SHA-256 mismatch' "$temporary/mismatch.err" || fail "model hash mismatch diagnostic"

runtime_test_dir=$temporary/llama-runtime
runtime_test_manifest=$temporary/llama-runtime.manifest
runtime_test_provenance=$temporary/embedding-provenance.json
mkdir -m 0755 "$runtime_test_dir"
cp "$runtime_manifest" "$runtime_test_manifest"
cp "$provenance_manifest" "$runtime_test_provenance"
chmod 0444 "$runtime_test_manifest"
chmod 0444 "$runtime_test_provenance"
runtime_test_verifier=$temporary/verify-llama-runtime
sed -e "s|^runtime_dir=.*|runtime_dir=$runtime_test_dir|" \
    -e "s|^manifest=.*|manifest=$runtime_test_manifest|" \
    -e "s|^provenance=.*|provenance=$runtime_test_provenance|" \
    -e "s|root:root:444|$test_user:$test_group:444|" \
    -e "s|root:root:755|$test_user:$test_group:755|" \
    -e "s|root:root:644|$test_user:$test_group:644|" \
    "$runtime_verifier" >"$runtime_test_verifier"
chmod 0700 "$runtime_test_verifier"

chmod 0644 "$runtime_test_provenance"
printf '\n' >>"$runtime_test_provenance"
chmod 0444 "$runtime_test_provenance"
if "$runtime_test_verifier" >"$temporary/provenance-tampered.out" 2>"$temporary/provenance-tampered.err"; then
    fail "runtime verifier accepted tampered provenance"
fi
grep -q 'embedding provenance manifest SHA-256 mismatch' "$temporary/provenance-tampered.err" || fail "provenance tamper diagnostic"
chmod 0644 "$runtime_test_provenance"
cp "$provenance_manifest" "$runtime_test_provenance"
chmod 0444 "$runtime_test_provenance"

if "$runtime_test_verifier" >"$temporary/runtime-missing.out" 2>"$temporary/runtime-missing.err"; then
    fail "runtime verifier accepted missing libraries"
fi
grep -q 'llama runtime library unavailable' "$temporary/runtime-missing.err" || fail "runtime missing-library diagnostic"

truncate -s 935472 "$runtime_test_dir/libggml-base.so.0"
chmod 0644 "$runtime_test_dir/libggml-base.so.0"
if "$runtime_test_verifier" >"$temporary/runtime-tampered.out" 2>"$temporary/runtime-tampered.err"; then
    fail "runtime verifier accepted tampered library"
fi
grep -q 'llama runtime library SHA-256 mismatch: libggml-base.so.0' "$temporary/runtime-tampered.err" || fail "runtime tamper diagnostic"

mount_root=$temporary/root
mkdir -p "$mount_root/etc"
printf '# existing crypttab\n' >"$mount_root/etc/crypttab"
printf '# existing fstab\n' >"$mount_root/etc/fstab"
install_output=$("$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$mount_root")
grep -q 'DRY-RUN append to .*etc/crypttab' <<<"$install_output" || fail "mount installer crypttab dry-run"
grep -q 'DRY-RUN append to .*etc/fstab' <<<"$install_output" || fail "mount installer fstab dry-run"
[[ $(wc -l <"$mount_root/etc/crypttab") -eq 1 ]] || fail "mount installer dry-run changed crypttab"
"$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$mount_root" --apply
grep -qxF 'neural-memory-data /var/lib/neural-memory/data.luks none tpm2-device=auto,discard,luks' "$mount_root/etc/crypttab" || fail "mount installer crypttab entry"
grep -qxF '/dev/mapper/neural-memory-data /srv/neural-memory-data ext4 nosuid,nodev,noexec 0 2' "$mount_root/etc/fstab" || fail "mount installer fstab entry"
[[ -f $mount_root/etc/crypttab.neural-memory-backup && -f $mount_root/etc/fstab.neural-memory-backup ]] || fail "mount installer backups"
crypt_lines=$(wc -l <"$mount_root/etc/crypttab")
fstab_lines=$(wc -l <"$mount_root/etc/fstab")
"$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$mount_root" --apply
[[ $(wc -l <"$mount_root/etc/crypttab") -eq $crypt_lines && $(wc -l <"$mount_root/etc/fstab") -eq $fstab_lines ]] || fail "mount installer idempotency"

absent_root=$temporary/absent-root
"$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$absent_root" --apply
grep -qxF 'neural-memory-data /var/lib/neural-memory/data.luks none tpm2-device=auto,discard,luks' "$absent_root/etc/crypttab" || fail "absent crypttab was not appended"
grep -qxF '/dev/mapper/neural-memory-data /srv/neural-memory-data ext4 nosuid,nodev,noexec 0 2' "$absent_root/etc/fstab" || fail "absent fstab was not appended"
[[ ! -e $absent_root/etc/crypttab.neural-memory-backup && ! -e $absent_root/etc/fstab.neural-memory-backup ]] || fail "absent files produced bogus backups"

assert_mount_conflict() {
    name=$1
    crypt_content=$2
    fstab_content=$3
    conflict_root=$temporary/conflict-$name
    mkdir -p "$conflict_root/etc"
    printf '%s\n' "$crypt_content" >"$conflict_root/etc/crypttab"
    printf '%s\n' "$fstab_content" >"$conflict_root/etc/fstab"
    crypt_before=$(sha256sum "$conflict_root/etc/crypttab")
    fstab_before=$(sha256sum "$conflict_root/etc/fstab")
    if "$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$conflict_root" --apply >/dev/null 2>&1; then
        fail "mount installer accepted conflict: $name"
    fi
    [[ $(sha256sum "$conflict_root/etc/crypttab") == "$crypt_before" ]] || fail "$name changed crypttab"
    [[ $(sha256sum "$conflict_root/etc/fstab") == "$fstab_before" ]] || fail "$name changed fstab"
}

assert_mount_conflict mapper 'neural-memory-data /wrong/source none luks' '# untouched'
assert_mount_conflict crypt-source 'other-mapper /var/lib/neural-memory/data.luks none luks' '# untouched'
assert_mount_conflict fstab-source '# empty' '/dev/mapper/neural-memory-data /other ext4 defaults 0 2'
assert_mount_conflict fstab-target '# empty' '/dev/other /srv/neural-memory-data ext4 defaults 0 2'

backup_root=$temporary/backup-collision
mkdir -p "$backup_root/etc"
printf '# existing crypttab\n' >"$backup_root/etc/crypttab"
printf '# existing fstab\n' >"$backup_root/etc/fstab"
printf '# occupied second backup\n' >"$backup_root/etc/fstab.neural-memory-backup"
crypt_before=$(sha256sum "$backup_root/etc/crypttab")
fstab_before=$(sha256sum "$backup_root/etc/fstab")
if "$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$backup_root" --apply >/dev/null 2>&1; then
    fail "mount installer accepted backup collision"
fi
[[ $(sha256sum "$backup_root/etc/crypttab") == "$crypt_before" ]] || fail "backup collision changed crypttab"
[[ $(sha256sum "$backup_root/etc/fstab") == "$fstab_before" ]] || fail "backup collision changed fstab"
[[ ! -e $backup_root/etc/crypttab.neural-memory-backup ]] || fail "backup collision created first backup"

canonical_root=$temporary/canonical-target
alternate_root=$temporary/canonical-parent/../canonical-target//
mkdir -p "$temporary/canonical-parent" "$canonical_root"
slash_output=$("$root/scripts/install-neural-memory-mount-entries-v1.sh" --root "$alternate_root")
grep -Fq "DRY-RUN append to $canonical_root/etc/crypttab:" <<<"$slash_output" || fail "alternate root crypttab path was not canonicalized"
grep -Fq "DRY-RUN append to $canonical_root/etc/fstab:" <<<"$slash_output" || fail "alternate root fstab path was not canonicalized"
! grep -Fq '/canonical-parent/../' <<<"$slash_output" || fail "alternate root retained parent traversal"
! grep -Fq '//etc/' <<<"$slash_output" || fail "alternate root retained redundant slash"
canonical_line=$(grep -n 'root=$(realpath -m -- "$root")' "$root/scripts/install-neural-memory-mount-entries-v1.sh" | cut -d: -f1)
privilege_line=$(grep -n 'root == / && $EUID -ne 0' "$root/scripts/install-neural-memory-mount-entries-v1.sh" | cut -d: -f1)
[[ -n $canonical_line && -n $privilege_line && $canonical_line -lt $privilege_line ]] || fail "root canonicalization must precede live privilege guard"

crypt_generator=/usr/lib/systemd/system-generators/systemd-cryptsetup-generator
fstab_generator=/usr/lib/systemd/system-generators/systemd-fstab-generator
if [[ -x $crypt_generator ]]; then
    generator_root=$temporary/crypt-generator
    mkdir -p "$generator_root"/{normal,early,late}
    grep -E '^[^#[:space:]]' "$crypttab" >"$generator_root/crypttab"
    SYSTEMD_CRYPTTAB=$generator_root/crypttab timeout 10s "$crypt_generator" \
        "$generator_root/normal" "$generator_root/early" "$generator_root/late"
    crypt_unit=$generator_root/normal/systemd-cryptsetup@neural\\x2dmemory\\x2ddata.service
    [[ -f $crypt_unit ]] || fail "cryptsetup generator did not create mapper unit"
    grep -Fq "'/var/lib/neural-memory/data.luks'" "$crypt_unit" || fail "cryptsetup generator stable source"
    grep -Fq "'tpm2-device=auto,discard,luks'" "$crypt_unit" || fail "cryptsetup generator options"
else
    echo "SKIP: systemd-cryptsetup-generator unavailable" >&2
fi
if [[ -x $fstab_generator ]]; then
    generator_root=$temporary/fstab-generator
    mkdir -p "$generator_root"/{normal,early,late,credentials}
    if ! grep -qxF '/dev/mapper/neural-memory-data /srv/neural-memory-data ext4 nosuid,nodev,noexec 0 2' /etc/fstab 2>/dev/null; then
        grep -E '^[^#[:space:]]' "$fstab" >"$generator_root/credentials/fstab.extra"
    fi
    CREDENTIALS_DIRECTORY=$generator_root/credentials SYSTEMD_SYSFS_CHECK=0 timeout 10s \
        "$fstab_generator" "$generator_root/normal" "$generator_root/early" "$generator_root/late"
    mount_unit=$generator_root/normal/srv-neural\\x2dmemory\\x2ddata.mount
    [[ -f $mount_unit ]] || fail "fstab generator did not create mount unit"
    grep -q '^What=/dev/mapper/neural-memory-data$' "$mount_unit" || fail "fstab generator mapper source"
    grep -q '^Where=/srv/neural-memory-data$' "$mount_unit" || fail "fstab generator mountpoint"
    grep -q '^Options=nosuid,nodev,noexec$' "$mount_unit" || fail "fstab generator mount options"
else
    echo "SKIP: systemd-fstab-generator unavailable" >&2
fi
provision_output=$("$root/scripts/provision-personal-volume-v1.sh" --image /var/lib/neural-memory/data.luks --recovery-key-file "$key")
grep -q '^DRY-RUN' <<<"$provision_output" || fail "provision dry-run"
chmod 0644 "$key"
if "$root/scripts/provision-personal-volume-v1.sh" --image /var/lib/neural-memory/data.luks --recovery-key-file "$key" >/dev/null 2>&1; then
    fail "provision accepted an unsafe recovery-key mode"
fi
chmod 0600 "$key"
if "$root/scripts/provision-personal-volume-v1.sh" --image "$temporary/data.luks" --recovery-key-file "$key" >/dev/null 2>&1; then
    fail "provision accepted a noncanonical backing path"
fi

mock_admin=$temporary/admin
printf '#!/bin/sh\nexit 99\n' >"$mock_admin"
chmod 0700 "$mock_admin"
source_db=$temporary/store.db
: >"$source_db"
migration_output=$("$root/scripts/migrate-evidence-to-encrypted-v1.sh" --source "$source_db" --admin "$mock_admin" --writers-stopped)
grep -q '^DRY-RUN' <<<"$migration_output" || fail "migration dry-run"

if "$root/scripts/migrate-evidence-to-encrypted-v1.sh" --source "$source_db" --admin "$mock_admin" >/dev/null 2>&1; then
    fail "migration accepted missing writers-stopped gate"
fi

dr_mock=$temporary/evidence-dr
printf '#!/bin/sh\nprintf "%%s\\n" "$*"\n' >"$dr_mock"
chmod 0700 "$dr_mock"
run_dr() {
    NEURAL_MEMORY_EVIDENCE_DR_BIN=$dr_mock \
    NEURAL_MEMORY_EVIDENCE_DR_DIR=/srv/neural-memory-data/backups/evidence-dr \
    SSH_ORIGINAL_COMMAND=$1 \
        "$root/scripts/evidence-dr-forced-command-v1.sh"
}
[[ $(run_dr list) == list ]] || fail "DR list grammar"
[[ $(run_dr 'stream manifest') == 'stream manifest' ]] || fail "DR stream grammar"
for rejected in 'stream ../store.db' 'stream personal.db' 'recall' 'list; id' 'stream backup extra'; do
    if run_dr "$rejected" >/dev/null 2>&1; then
        fail "DR wrapper accepted: $rejected"
    fi
done

sync_mock=$temporary/personal-sync
printf '#!/bin/sh\nprintf "%%s\\n" "$*"\n' >"$sync_mock"
chmod 0700 "$sync_mock"
mounted_mock=$temporary/mountpoint-mounted
printf '#!/bin/sh\nexit 0\n' >"$mounted_mock"
chmod 0700 "$mounted_mock"
missing_mock=$temporary/mountpoint-missing
printf '#!/bin/sh\nexit 1\n' >"$missing_mock"
chmod 0700 "$missing_mock"
sync_wrapper_mounted=$temporary/personal-sync-mounted
sed "s|/usr/bin/mountpoint|$mounted_mock|g" "$root/scripts/personal-sync-forced-command-v1.sh" >"$sync_wrapper_mounted"
chmod 0700 "$sync_wrapper_mounted"
sync_wrapper_missing=$temporary/personal-sync-missing
sed "s|/usr/bin/mountpoint|$missing_mock|g" "$root/scripts/personal-sync-forced-command-v1.sh" >"$sync_wrapper_missing"
chmod 0700 "$sync_wrapper_missing"
run_sync() {
    NEURAL_MEMORY_PERSONAL_SYNC_BIN=$sync_mock \
    NEURAL_MEMORY_PERSONAL_DB=/srv/neural-memory-data/personal/personal.db \
    NEURAL_MEMORY_PERSONAL_KEY=/srv/neural-memory-data/keys/gpd-ed25519.seed \
    NEURAL_MEMORY_MAC_PUBLIC_KEY=/srv/neural-memory-data/keys/mac-ed25519.pub \
    NEURAL_MEMORY_PERSONAL_DEVICE=gpd-win-mini \
    SSH_ORIGINAL_COMMAND=$1 \
        "$sync_wrapper_mounted"
}
[[ $(run_sync status) == '{"health":"ready"}' ]] || fail "sync mounted status"
[[ $(run_sync public-key) == public-key ]] || fail "sync public-key grammar"
[[ $(run_sync 'export --after 1:2') == 'export --after 1:2' ]] || fail "sync export grammar"
[[ $(run_sync 'acknowledge --through 1:2') == 'acknowledge --through 1:2' ]] || fail "sync acknowledge grammar"
[[ $(run_sync import) == import ]] || fail "sync import grammar"
run_sync_missing() {
    NEURAL_MEMORY_PERSONAL_SYNC_BIN=$sync_mock \
    NEURAL_MEMORY_PERSONAL_DB=/srv/neural-memory-data/personal/personal.db \
    NEURAL_MEMORY_PERSONAL_KEY=/srv/neural-memory-data/keys/gpd-ed25519.seed \
    NEURAL_MEMORY_MAC_PUBLIC_KEY=/srv/neural-memory-data/keys/mac-ed25519.pub \
    NEURAL_MEMORY_PERSONAL_DEVICE=gpd-win-mini \
    SSH_ORIGINAL_COMMAND=$1 \
        "$sync_wrapper_missing"
}
[[ $(run_sync_missing status) == '{"health":"blocked-on-mount"}' ]] || fail "sync missing-mount status"
for blocked in 'export --after 1:2' 'import'; do
    set +e
    blocked_output=$(run_sync_missing "$blocked" 2>"$temporary/sync-blocked.err")
    blocked_status=$?
    set -e
    [[ $blocked_status -eq 75 ]] || fail "sync missing mount did not use blocked status: $blocked"
    [[ -z $blocked_output ]] || fail "sync child ran while mount missing: $blocked"
    grep -q 'personal data mount unavailable' "$temporary/sync-blocked.err" || fail "sync missing-mount diagnostic"
done
for rejected in '' 'recall' 'sql select' 'export --after ../store.db' 'export --after 1:2 extra' 'public-key; id' 'sh -c id'; do
    if run_sync "$rejected" >/dev/null 2>&1; then
        fail "sync wrapper accepted: $rejected"
    fi
done

env_mock=$temporary/env-child
printf '#!/bin/sh\n/usr/bin/env | /usr/bin/sort\nprintf "ARGS=%%s\\n" "$*"\n' >"$env_mock"
chmod 0700 "$env_mock"
sync_config=$temporary/personal-sync.conf
printf 'NEURAL_MEMORY_PERSONAL_SYNC_BIN=%s\nNEURAL_MEMORY_PERSONAL_DB=/fixed/personal.db\nNEURAL_MEMORY_PERSONAL_KEY=/fixed/signing.key\nNEURAL_MEMORY_MAC_PUBLIC_KEY=/fixed/mac.pub\nNEURAL_MEMORY_PERSONAL_DEVICE=gpd-test\n' "$env_mock" >"$sync_config"
chmod 0640 "$sync_config"
sync_from_config=$temporary/personal-sync-wrapper
sed -e "s|config=/etc/neural-memory/personal-sync.conf|config=$sync_config|" \
    -e "s|0:neural-memory:640|$test_uid:$test_group:640|" \
    -e "s|/usr/bin/mountpoint|$mounted_mock|g" \
    "$root/scripts/personal-sync-forced-command-v1.sh" >"$sync_from_config"
chmod 0700 "$sync_from_config"
sync_env_output=$(/usr/bin/env -i SSH_ORIGINAL_COMMAND=public-key /bin/bash "$sync_from_config")
grep -qxF 'NEURAL_MEMORY_PERSONAL_DB=/fixed/personal.db' <<<"$sync_env_output" || fail "sync DB env propagation"
grep -qxF 'NEURAL_MEMORY_PERSONAL_KEY=/fixed/signing.key' <<<"$sync_env_output" || fail "sync key env propagation"
grep -qxF 'NEURAL_MEMORY_MAC_PUBLIC_KEY=/fixed/mac.pub' <<<"$sync_env_output" || fail "sync peer key env propagation"
grep -qxF 'NEURAL_MEMORY_PERSONAL_DEVICE=gpd-test' <<<"$sync_env_output" || fail "sync device env propagation"
grep -qxF 'ARGS=public-key' <<<"$sync_env_output" || fail "sync config child arguments"
[[ $(grep -c '^NEURAL_MEMORY_' <<<"$sync_env_output") -eq 4 ]] || fail "sync child received extra neural-memory environment"
! grep -q '^SSH_ORIGINAL_COMMAND=' <<<"$sync_env_output" || fail "sync child received SSH_ORIGINAL_COMMAND"
if /usr/bin/env -i SSH_ORIGINAL_COMMAND='public-key; id' /bin/bash "$sync_from_config" >/dev/null 2>&1; then
    fail "config-backed sync wrapper accepted malformed command"
fi

dr_config=$temporary/evidence-dr.conf
printf 'NEURAL_MEMORY_EVIDENCE_DR_BIN=%s\nNEURAL_MEMORY_EVIDENCE_DR_DIR=/fixed/evidence-dr\n' "$env_mock" >"$dr_config"
chmod 0640 "$dr_config"
dr_from_config=$temporary/evidence-dr-wrapper
sed -e "s|config=/etc/neural-memory/evidence-dr-pull.conf|config=$dr_config|" \
    -e "s|0:neural-memory:640|$test_uid:$test_group:640|" \
    "$root/scripts/evidence-dr-forced-command-v1.sh" >"$dr_from_config"
chmod 0700 "$dr_from_config"
dr_env_output=$(/usr/bin/env -i SSH_ORIGINAL_COMMAND=list /bin/bash "$dr_from_config")
grep -qxF 'NEURAL_MEMORY_EVIDENCE_DR_DIR=/fixed/evidence-dr' <<<"$dr_env_output" || fail "DR directory env propagation"
grep -qxF 'ARGS=list' <<<"$dr_env_output" || fail "DR config child arguments"
[[ $(grep -c '^NEURAL_MEMORY_' <<<"$dr_env_output") -eq 1 ]] || fail "DR child received extra neural-memory environment"
! grep -q '^SSH_ORIGINAL_COMMAND=' <<<"$dr_env_output" || fail "DR child received SSH_ORIGINAL_COMMAND"
if /usr/bin/env -i SSH_ORIGINAL_COMMAND='list; id' /bin/bash "$dr_from_config" >/dev/null 2>&1; then
    fail "config-backed DR wrapper accepted malformed command"
fi

if grep -q 'personal\.db' "$root/crates/neural-memory-personal/src/evidence_dr.rs"; then
    fail "evidence DR library references personal.db"
fi

echo "PASS: deployment artifacts are fail-closed and dry-run safe"
