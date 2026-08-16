# GPD encrypted deployment plan

These files are reviewable templates, not a live-system installer. Do not use
`--apply`, install units, edit SSH, disable swap, or enroll TPM until the GPD is
on console, backups and the recovery key have been tested, and Tailscale is
enrolled. All database paths are fixed below `/srv/neural-memory-data`.

## Encrypted volume

`scripts/provision-personal-volume-v1.sh` is dry-run by default. Its apply mode
creates a 16 GiB sparse image at `/var/lib/neural-memory/data.luks`, attaches a
transient loop device, formats it as LUKS2,
opens it with cryptsetup `--allow-discards`, creates ext4, and mounts it at
`/srv/neural-memory-data`. The filesystem itself is mounted without continuous
`discard`; weekly `fstrim` is used instead.

Supply a separately generated recovery-key file with mode `0400` or `0600`.
Keep a tested offline copy away from the GPD. The script never prints, creates,
or deletes that key. After recovery-key unlock has been tested, enroll TPM2 from
the console using a transient loop attachment. The loop name is discovered for
this operation and is never copied into boot configuration:

```sh
loop_device=$(sudo losetup --find --show --direct-io=on /var/lib/neural-memory/data.luks)
sudo systemd-cryptenroll --unlock-key-file=/path/to/offline-recovery.key \
  --tpm2-device=auto --tpm2-pcrs=7+11 "$loop_device"
sudo cryptsetup luksDump "$loop_device"
sudo losetup --detach "$loop_device"
```

Confirm the PCR policy against the installed boot chain before enrollment.
Retain the recovery slot. Systemd 259 reads `/etc/crypttab`; it does not read a
`/etc/crypttab.d` directory. Review the entries in
`deploy/crypttab/neural-memory-data.conf` and
`deploy/fstab/neural-memory-data.conf`, then run
`scripts/install-neural-memory-mount-entries-v1.sh` in dry-run mode. Its apply
mode appends to `/etc/crypttab` and `/etc/fstab`, never installs an ignored
drop-in. It first rejects conflicting mapper, source, or mountpoint entries and
backs up existing files before changing either. Re-running an exact installation
is a no-op. Systemd opens the stable file
path with TPM2 and discard and creates its own transient loop attachment. The
ext4 mount uses `nosuid,nodev,noexec`; no `/dev/loopN` name is persisted.
Dependent units retain both `RequiresMountsFor=` and
`ConditionPathIsMountPoint=`, so failed unlock or mount prevents service start.
Acceptance checks are:

```sh
findmnt -no SOURCE,OPTIONS /srv/neural-memory-data
cryptsetup status neural-memory-data
stat -c '%a %U %G %n' /srv/neural-memory-data/{personal,evidence,backups,staging,sqlite-tmp,outboxes,keys,logs}
sudo -u neural-memory test -w /srv/neural-memory-data/personal
sudo fstrim --verbose /srv/neural-memory-data
```

Every layout directory must be owned by `neural-memory:neural-memory` with mode
`0700`. Database and key files must be `0600`. `fstrim.timer` should be enabled
only after `fstrim --verbose` succeeds on this mapping. The provided timer
drop-in schedules weekly trim; it is not discard-on-every-write.

## Evidence migration

Evidence remains a separate `store.db`; it is never imported into `personal.db`.
Stop and verify all evidence writers first, then run
`scripts/migrate-evidence-to-encrypted-v1.sh` in dry-run mode. Apply mode uses
the repository's `neural-memory-admin backup`, which performs SQLite `VACUUM
INTO` and content/count/integrity verification, then verifies the installed copy
again. It refuses occupied or existing destinations. Both the plaintext source
and verified staging copy remain until a human accepts the encrypted service.
Removal later is a separate, explicit operation and is not part of these files.
The migration process sets `umask 077` before invoking the admin backup. After
the staging copy passes the existing verifier, it is retained as
`neural-memory:neural-memory` mode `0600`; those attributes are asserted before
the mode-`0600` destination is installed and verified again.

Do not run ordinary writable `sqlite3 ... PRAGMA integrity_check` against the
retained plaintext source before hashing it. Opening a WAL database read/write
can checkpoint committed WAL pages into the main file and change its bytes even
after application writers have stopped. Before any SQLite open, hash the main
file and WAL independently. Hash the SHM sidecar too when present for retention
auditing, but do not treat it as durable database content:

```sh
sha256sum /plaintext/store.db > store.db.before.sha256
[[ ! -e /plaintext/store.db-wal ]] || sha256sum /plaintext/store.db-wal > store.db-wal.before.sha256
[[ ! -e /plaintext/store.db-shm ]] || sha256sum /plaintext/store.db-shm > store.db-shm.before.sha256
```

Only when no `store.db-wal` exists after an explicitly accepted clean
checkpoint may the main file be inspected without write capability using
`sqlite3 'file:/plaintext/store.db?mode=ro&immutable=1' 'PRAGMA integrity_check;'`.
`immutable=1` and nolock semantics can omit uncheckpointed WAL and must never be
claimed as validation of a complete database while a WAL sidecar exists. Do not
substitute a standalone `sqlite3 integrity_check` for a WAL-aware coherent
check. The repository `neural-memory-admin backup` (`VACUUM INTO`) plus
`verify-backup` is the authoritative coherent verification flow.

After that flow, hash each same source component independently again and compare
it with its corresponding before hash. Any changed or missing component fails
source-preservation acceptance. Retain the plaintext main file and every
existing `-wal` and `-shm` sidecar until explicit acceptance; these hashes audit
preservation and do not replace the evidence verifier.

## Signed evidence disaster recovery

`neural-memory-evidence-dr` wraps—but does not replace—the existing evidence
backup contract. Its fixed paths are:

- source: `/srv/neural-memory-data/evidence/store.db`
- staging: `/srv/neural-memory-data/backups/evidence-dr/staging`
- accepted pull set: `/srv/neural-memory-data/backups/evidence-dr/current`
- signing key: `/srv/neural-memory-data/keys/gpd-ed25519.seed`

The operator-only `stage --writers-stopped --created-at TIMESTAMP` command also
requires `NEURAL_MEMORY_EVIDENCE_WRITERS_STOPPED=1`. It invokes
`neural-memory-admin backup` without `--no-verify`, requires its verified report,
then invokes `neural-memory-admin verify-backup` again. Only after both commands
succeed does it hash and sign the manifest. A failed verifier leaves the
unsigned staging database for inspection. It never opens `personal.db`.

The literal UTF-8 manifest bytes have this exact schema and field order:

```json
{"version":"EvidenceBackupManifestV1","artifactFilename":"evidence-current.db","byteLength":123456,"sha256":"LOWERCASE_SHA256","createdAt":"2026-08-06T12:34:56.789Z","evidenceVerifierSuccess":true,"sourceCounts":{"records":10,"observations":20,"edges":9,"schemaVersion":1},"backupCounts":{"records":10,"observations":20,"edges":9,"schemaVersion":1}}
```

Because the evidence verifier reports exact equality, the verified backup
counts equal the source counts. The signed envelope uses the existing Ed25519
semantics—signature over those literal manifest bytes, lowercase SHA-256 key
ID, and base64 payload/signature:

```json
{"version":"SignedEvidenceBackupManifestV1","algorithm":"Ed25519","signerKeyID":"LOWERCASE_SHA256_PUBLIC_KEY","payloadBase64":"BASE64_LITERAL_MANIFEST","signatureBase64":"BASE64_ED25519_SIGNATURE"}
```

`accept --trusted-key-base64 KEY` receives the explicitly enrolled GPD public
key. It verifies signature first, then fixed filename, canonical timestamp,
verifier result/count equality, byte length, and SHA-256. It copies the three
fixed staging files into `current.next`, verifies that independent copy, and
atomically renames the directory to `current`. It refuses to replace an existing
current set. The live source and staging files remain after acceptance.

The separate forced-command account exposes only:

```text
list
stream backup
stream manifest
stream signature
```

There is no caller-provided path or glob and no recall, SQL, personal-memory,
shell, or general filesystem command. Regular-file/inode checks reject symlinks
and detect replacement while opening. Install the reviewed wrapper and
`authorized_keys.evidence-dr.example` only after tailnet enrollment.

On the Mac, DR ingestion must occur in this order:

1. Fetch `list`, then the fixed manifest and signature objects.
2. Verify the Ed25519 signature over the literal decoded manifest bytes using
   the enrolled GPD public key before parsing or trusting metadata.
3. Require the fixed version and filename `evidence-current.db`.
4. Stream the backup to a new private staging file; verify exact byte length and
   SHA-256.
5. Run SQLite `PRAGMA integrity_check` and the same-version evidence
   `neural-memory-admin verify-backup` against the Mac evidence source.
6. Only after all checks succeed, atomically enter the artifact into the Mac DR
   retention set. Never import evidence into personal memory.

Mac retention implementation and live cross-device restoration remain outside
this GPD phase.

## Signed interaction-logs disaster recovery

`neural-memory-interaction-logs-dr` is the sibling of the evidence DR flow —
same signer, same envelope semantics, same four-word forced-command surface —
for a different artifact: a deterministic GNU tar of the interaction logs (raw
EEG captures, turn logs, session records, and the derived `.workspace/`
artifacts). Its fixed paths are:

- source: `/srv/neural-memory-data/interaction-logs`
- staging: `/srv/neural-memory-data/backups/interaction-logs-dr/staging`
- accepted pull set: `/srv/neural-memory-data/backups/interaction-logs-dr/current`
- signing key: `/srv/neural-memory-data/keys/gpd-ed25519.seed`
- archiver: `/usr/bin/tar` (`--format=gnu --sort=name --mtime=@0 --owner=0
  --group=0 --numeric-owner`, so identical content produces an identical
  archive)

The operator-only `stage --writers-stopped --created-at TIMESTAMP` command also
requires `NEURAL_MEMORY_INTERACTION_LOGS_WRITERS_STOPPED=1`. Staging walks the
source (rejecting symlinks and non-regular files outright), hashes every file,
builds the archive, then verifies **by extraction**: the archive is unpacked
into a scratch directory and every member's SHA-256 compared against the
pre-tar walk. That double hash is also the writers check in practice — a
capture growing between walk and tar fails the stage and leaves the staging
directory for inspection. In operational terms, writers-stopped means no
hypnagogic session is running.

The manifest (`InteractionLogsBackupManifestV1`) carries the archive's byte
length and SHA-256, `createdAt`, `fileCount`, `contentBytes`, and a sorted
per-file `{path, byteLength, sha256}` listing so the Mac can verify individual
members after extraction, not just the archive. The signed envelope
(`SignedInteractionLogsBackupManifestV1`) uses the existing Ed25519 semantics —
signature over the literal manifest bytes, lowercase SHA-256 key ID, base64
payload and signature. `accept --trusted-key-base64 KEY` verifies signature
first, then filename, canonical timestamp, file-count and content-bytes
consistency, path safety (no absolute or `..` members), byte length, and
SHA-256; copies the three fixed staging files into `current.next`; verifies the
independent copy; atomically renames to `current`; and refuses to replace an
existing current set.

The separate forced-command account
(`scripts/interaction-logs-dr-forced-command-v1.sh`, config
`/etc/neural-memory/interaction-logs-dr-pull.conf`) exposes only `list`,
`stream backup`, `stream manifest`, `stream signature` — no caller-provided
path or glob, same rejection character class as the evidence wrapper. Mac
ingestion follows the same order as evidence DR: fetch, verify the Ed25519
signature over the literal manifest bytes before parsing, then verify the
archive hash and per-file hashes after extraction.

## Services and network confinement

The HTTPS transport remains a system service using
`RequiresMountsFor=/srv/neural-memory-data`, a fixed path, an unprivileged
account, and filesystem sandboxing. A missing encrypted mount prevents it from
starting. Stdio MCP servers are not systemd daemons: they require a client's
stdin/stdout and are launched by the local MCP client for each session.

Install `scripts/neural-memory-personal-mcp-v1.sh` and
`scripts/neural-memory-evidence-mcp-v1.sh` under `/usr/local/libexec` as
root-owned mode `0755`, and install `deploy/sudoers/neural-memory-local-mcp` as
root-owned mode `0440` only after `visudo -cf` succeeds. The policy allows only
local user `aurascoper` to run either fixed wrapper as `neural-memory`, with no
caller arguments. Configure the local MCP client command as one of:

```text
/usr/bin/sudo -n -u neural-memory /usr/local/libexec/neural-memory-personal-mcp-v1
/usr/bin/sudo -n -u neural-memory /usr/local/libexec/neural-memory-evidence-mcp-v1
```

The wrappers require the encrypted mount, clear the caller environment, and
accept no arguments. They set `umask 077` before any possible file creation;
system services that can create personal databases, keys, tokens, outboxes, or
logs use `UMask=0077`. Personal MCP opens only
`/srv/neural-memory-data/personal/personal.db`. Evidence MCP opens only
`/srv/neural-memory-data/evidence/store.db` and generates its UTC `--as-of` at
launch. The sudo policy exposes no arbitrary binary, shell, SQL, path, or remote
recall surface.

The personal store also enforces this boundary itself: a new `personal.db` is
precreated as mode `0600`, while an existing database must be a regular,
non-symlink file owned by the effective user with exact mode `0600`. An
insecure existing database fails closed instead of being silently reused.

The SSH templates are forced-command-only with `restrict` and tailnet `from=`
ranges. Replace the sample ranges after enrollment; do not install them before
then. Create locked, non-login transport accounts `neural-memory-sync` and
`neural-memory-dr`. Their root-managed homes contain only their
`.ssh/authorized_keys` files, and they are not members of the `neural-memory`
group. They therefore cannot traverse the mode-`0700`, `neural-memory`-owned
encrypted layout or directly read configs, databases, or keys.

Install the corresponding files from `deploy/sudoers/` as root-owned mode
`0440` only after `visudo -cf` succeeds. Each account has one `NOPASSWD` rule:
run its fixed wrapper with no arguments as user `neural-memory`. There is no
shell, editor, wildcard, arbitrary-argument, root, or general sudo permission.
Command-scoped `env_keep` preserves only `SSH_ORIGINAL_COMMAND`; `env_reset`
drops caller configuration. The forced command does not interpolate the
original command. The wrapper independently rejects metacharacters, unknown
arguments, recall, SQL, and caller paths before invoking a fixed binary.

Install `config/personal-sync.conf.example` and
`config/evidence-dr-pull.conf.example` at their documented fixed paths as
`root:neural-memory` mode `0640`. The wrappers reject symlink configs or any
other owner/group/mode. These configs are readable after sudo but not by either
SSH account. Keep wrappers and binaries root-owned and unwritable by
`neural-memory` and the SSH accounts.

The personal-sync forced command exposes one data-free mount preflight:
`status`. It emits exactly `{"health":"ready"}` when
`/srv/neural-memory-data` is an active mount, or
`{"health":"blocked-on-mount"}` otherwise; both are successful protocol
responses. With the mount absent, export, acknowledge, import, and key access
fail closed before the Rust binary or `personal.db` is touched. Mac coordinator
implementation remains outside this GPD repository.

Provision `/srv/neural-memory-data/keys/mac-ed25519.pub` only through the local,
confirmation-gated `enroll-peer` operation. It is raw 32-byte Ed25519 public-key
material, mode `0600`, owned by `neural-memory`, and loaded with no-symlink and
regular-file checks. Forced SSH and HTTPS imports never accept a caller key.

The HTTPS fallback is the same signed `SyncBundleV1` protocol behind mTLS. Its
nginx template binds to a placeholder tailnet address, accepts only `POST` on
one exact path, and proxies to `neural-memory-personal-transport` on loopback.
The backend independently requires canonical base64 encoding of exactly 32
random bytes from the mode-`0600` file
`/srv/neural-memory-data/keys/personal-transport.token`. It
refuses non-loopback listen addresses and permissive token files.

Nginx deliberately drops client request headers and supplies the backend
credential through this separately provisioned, mode-`0600` encrypted include:

```nginx
proxy_set_header Authorization "Bearer REPLACE_WITH_THE_TOKEN_FILE_VALUE";
```

Store that one-line file at
`/srv/neural-memory-data/keys/nginx-personal-sync-bearer.conf`; never put its
contents in this repository or logs. The mTLS client certificate remains the
external client identity, while the bearer credential confines direct backend
access. Certificate and key paths remain operator-provided placeholders.

### HTTPS JSON protocol

Every request is `POST /v1/personal-sync`, `Content-Type: application/json`, no
larger than 8 MiB, with the nginx-supplied `Authorization: Bearer TOKEN`. Request
objects reject unknown fields and have exactly one of these shapes:

```json
{"action":"status"}
{"action":"publicKey"}
{"action":"export","after":"1:42"}
{"action":"acknowledge","through":"1:42"}
{"action":"import","envelope":{"version":"SyncBundleV1","algorithm":"Ed25519","signerKeyID":"LOWERCASE_SHA256","payloadBase64":"BASE64_JSON_BYTES","signatureBase64":"BASE64_SIGNATURE"}}
```

Successful responses exactly match the sync CLI JSON semantics:

```json
{"health":"ready"}
{"algorithm":"Ed25519","signerKeyID":"LOWERCASE_SHA256","publicKeyBase64":"BASE64_ED25519_PUBLIC_KEY"}
{"version":"SyncBundleV1","algorithm":"Ed25519","signerKeyID":"LOWERCASE_SHA256","payloadBase64":"BASE64_JSON_BYTES","signatureBase64":"BASE64_SIGNATURE"}
{"acknowledgedThrough":{"epoch":1,"sequence":42},"promoted":3}
{"ack":{"through":{"epoch":1,"sequence":42},"committed":true}}
```

Errors are JSON with a non-2xx status:

```json
{"error":{"code":"invalidRequest","message":"bounded diagnostic without request content"}}
```

HTTPS `status` is authenticated and data-free: it does not open `personal.db`
or the signing key. The transport unit has both `RequiresMountsFor` and
`ConditionPathIsMountPoint`, however, so it may be stopped and unreachable when
the encrypted mount is absent. In that state, `blocked-on-mount` remains
observable through the forced-command SSH `status`; HTTPS unreachability is a
transport outcome, not a blocked-status response.

Only `status`, `publicKey`, `export`, `acknowledge`, and `import` exist. There is no HTTP
recall, MCP, SQL, filesystem, or shell surface. Tokens and bundle bodies are
never logged.

## zram and controlled reboot

The zram-generator template requests 12 GiB zstd swap. Review available RAM and
install the template, disable hibernation targets, then perform a controlled
reboot. Verify before changing plaintext swap:

```sh
zramctl
swapon --show --bytes
systemctl status systemd-zram-setup@zram0.service
systemctl mask hibernate.target hybrid-sleep.target suspend-then-hibernate.target
```

Only after the 12 GiB zram device is active across reboot and memory-pressure
testing succeeds may an operator `swapoff` and remove plaintext swap from
`fstab`. Do not disable the fallback first. Hibernation stays disabled because
resume would require persistent plaintext or separately protected swap. Do not
mask `sleep.target` or `suspend.target`: ordinary suspend remains required. Test
one controlled suspend/resume cycle and then re-check the encrypted mount,
personal services, zram, and absence of plaintext swap.

## Threat model

The encrypted mount addresses stolen-device offline access to personal memory,
evidence copies, keys, outboxes, and logs. It does not make previously written
plaintext disappear: deleting plaintext originals later is not secure erasure
on NAND/SSD media.

Tamper-and-return remains an accepted residual risk in this phase. The root
filesystem and service binaries are plaintext and Secure Boot is off, so an
attacker with the device can modify software and capture data after unlock. A
full-disk LUKS reinstall with a verified Secure Boot chain is required to close
that gap; the loopback volume cannot.
