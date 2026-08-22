# Personal memory: GPD foundation

This component uses `personal.db`, never the evidence `store.db`.

## Banking policy

- The Mac is canonical.
- The GPD keeps local captures plus a canonical replica received from the Mac.
- There is no live remote recall. Recall on the GPD reads `personal.db` only.
- Evidence records are never promoted into personal memory.
- Never auto-bank secrets, raw biometric data, audio, EEG, medical data,
  third-party content, or transient model/tool outputs.

## Sync command confinement

The signing seed path is explicit and is intended to be inside the future
encrypted mount. The sync CLI creates it as mode `0600` and refuses a more
permissive existing file. The Mac public key is enrolled locally into the fixed
mode-`0600` encrypted-mount file named by `NEURAL_MEMORY_MAC_PUBLIC_KEY` using
`enroll-peer --public-key-base64 KEY --confirm ENROLL-MAC-PEER`. Imports load
only that file; requests and bundles never select a trusted key.

Install `scripts/personal-sync-forced-command-v1.sh` with the fixed
root-owned configuration from `config/personal-sync.conf.example`. The
dedicated, locked `neural-memory-sync` SSH account uses the reviewed sudoers
rule to run only that wrapper as `neural-memory`; it is not granted encrypted
data access. Its `authorized_keys` entry combines the fixed sudo invocation
with OpenSSH restrictions and tailnet-only `from=` ranges:

```text
restrict,from="100.64.0.0/10,fd7a:115c:a1e0::/48",command="/usr/bin/sudo -n -u neural-memory /usr/local/libexec/personal-sync-forced-command-v1" ssh-ed25519 AAAA...
```

Replace those ranges with the enrolled tailnet ranges actually assigned to the
Mac. Do not install the entry or edit `sshd_config` until Tailscale enrollment
is complete and the source ranges have been verified.

The wrapper allows only `status`, `public-key`, `export --after E:S`, `acknowledge
--through E:S`, and bare `import`. It exposes no enrollment, recall,
SQL, shell, or filesystem command.

`status` is the sole operation available while the encrypted data mount is
absent. It never invokes the sync binary and therefore never opens or creates
`personal.db`, a signing key, or any other encrypted-path file. It exits zero
with exactly one of these JSON objects:

```json
{"health":"ready"}
{"health":"blocked-on-mount"}
```

The command grammar is the single token `status`, with no arguments. When
`/srv/neural-memory-data` is not an active mount, every other otherwise-valid
operation is refused with exit status 75 and a diagnostic on stderr. Malformed
or forbidden commands remain grammar failures with exit status 2. This lets a
Mac coordinator distinguish a successful blocked preflight from transport and
protocol failures without exposing recall, SQL, paths, filesystem access, or
unrestricted MCP. The Mac coordinator itself is outside this repository.

The authenticated HTTPS fallback also accepts exactly
`{"action":"status"}` and returns `{"health":"ready"}` without opening the
database or signing key. Its systemd unit is mount-dependent, so HTTPS may be
unreachable rather than returning blocked status when the mount is absent;
forced-command SSH is the authoritative `blocked-on-mount` observation path.

The reviewed GPD provisioning, service, trim, zram, migration, and threat-model
templates are described in [gpd-deployment.md](gpd-deployment.md). They are not
installed or applied by this repository.

## Local personal embeddings and status

Personal vectors are stored only in `personal.db`. They are never serialized in
`SyncBundleV1`; both local captures and imported canonical Mac records enqueue
local GPD derivation in the same database transaction. Tombstones remove queued
and ready vectors. The four-tool personal MCP remains unchanged. Embedding,
status, health, and context operations are available only through the local
operator binary `neural-memory-personal-admin`.

An embedding profile identity is SHA-256 over RFC 8785 canonical JSON containing
exactly `backend`, `modelArtifact`, `dimension`, `normalization`, `version`,
`adapter`, and `endpoint` (including `null` versus a value). A vector is keyed
by `(profileIdentity, contentDigest)`. Only the
active profile is ready; vectors from older profiles are reported stale and are
never treated as current or compared with another profile.

Production uses the llama.cpp OpenAI-compatible local embedding endpoint. The
strict measured GPD profile is:

- backend `llama.cpp-cpu`
- llama.cpp source commit `d0bfb1981266c271cd0536a8aa7c5e863e7cdf61`;
  `--version` reports build 10188 and short commit `d0bfb1981`
- model `nomic-embed-text-v1.5.Q8_0.gguf` from
  `nomic-ai/nomic-embed-text-v1.5-GGUF` at source revision
  `18d1044f4866e224159fce8c6fc5c4f3920176e7`, exactly 146146432 bytes,
  SHA-256 `3e24342164b3d94991ba9692fdc0dd08e3fd7362e0aacc396a9a5c54a544c3b7`
- endpoint `http://127.0.0.1:8082`, dimension 768, normalization `l2`, adapter
  `llama-cpp-http`
- this exact GGUF declares a 2048-token training context; context, logical
  batch, and physical batch are therefore explicitly 2048
- parallelism is explicitly one, giving the serial worker the full 2048-token
  slot while equal batches avoid llama.cpp's embedding fallback to 512

The live CPU measurement produced finite L2-normalized vectors, a 26 ms probe,
and approximately 131 MiB RSS. These are observations, not startup thresholds.
Install the root-owned server at `/usr/local/bin/llama-server` mode `0755`,
exactly 17904 bytes with SHA-256
`bd95aacd01abd53a2eed1a08c3e37f808d1760e7161ca7f5520a452ff81c0fe2`, and
the `root:neural-memory` model at
`/usr/local/share/neural-memory/models/nomic-embed-text-v1.5.Q8_0.gguf` mode
`0640`; neither artifact is stored in this repository.

`neural-memory-embedding-server.service` is loopback-only and CPU-only. Its
fixed preflights reject a missing/replaced model, size or hash mismatch, unsafe
metadata, wrong server build, or any missing/extra/tampered runtime library
before startup. The seven-library CPU runtime is packaged as regular root-owned
files under `/usr/local/lib/neural-memory/llama-cpu`; the service sets that exact
`LD_LIBRARY_PATH` and retains `ProtectHome=true`, so the build-tree RUNPATH is
neither accessible nor required. The exact manifest and install commands are in
`deploy/embedding/`. A bounded fixed `ExecStartPost` probes the exact loopback
embedding operation for up to 90 seconds, so the service is not considered
started while llama.cpp still returns HTTP 503 during model loading. Its fixed
1800-word probe measured approximately 1803 live tokens, within the artifact's
2048 context while still beyond the former 512-token batch ceiling. The helper
accepts no arguments and cannot be redirected to another endpoint. The
mount-dependent periodic
worker activates the exact profile idempotently and rebuilds at most 100 queued
local/imported active records per run with a canonical current timestamp.
First activation and rotation queue all active records; identical reactivation
does not requeue ready records. Profile arguments remain repeated at rebuild so
a mismatch fails rather than mixing spaces.
An explicitly over-limit record is recorded locally as stale and removed from
the queue so later records proceed; transient endpoint failures remain queued
and fail the worker. Batch and context capacity do not alter vectors for inputs
accepted by both configurations, so they are deployment policy rather than
embedding profile identity fields.

Every record returned by personal MCP `recall`/`list_recent` or local `context`
includes its per-record semantic availability. Lexical retrieval never hides a
record merely because its local vector is unavailable:

```json
{"semanticBranch":{"ran":true,"profileIdentity":"<sha256>"}}
{"semanticBranch":{"ran":false,"reason":"input-too-large","profileIdentity":"<sha256>"}}
{"semanticBranch":{"ran":false,"reason":"pending-local-embedding","profileIdentity":"<sha256>"}}
{"semanticBranch":{"ran":false,"reason":"no-active-profile"}}
```

Terminal failure rows store only profile identity, record digest, the closed
reason, and failure timestamp. They never duplicate memory text.

The current upstream model card advertises 8192 tokens, but that does not prove
the provenance or capability of this exact GGUF. Its loaded metadata and live
`n_ctx_train=2048` behavior are authoritative. The sealed
`deploy/embedding/embedding-provenance-v1.json` records the source repository,
full source commit, build/version, toolchain and flags, model upstream URL with
an explicitly unknown revision, and exact artifact hashes. Startup verification
checks the literal provenance-manifest bytes rather than relying on this prose.

The deterministic adapter is test-only and requires the explicit
`--allow-test-backend yes` gate; it is not a production substitute.

Local inspection commands are bounded and open only the named `personal.db`:

```sh
neural-memory-personal-admin status \
  --db /srv/neural-memory-data/personal/personal.db
neural-memory-personal-admin context \
  --db /srv/neural-memory-data/personal/personal.db \
  --query jasmine --limit 20
```

Status contains no memory text. It reports the DB path and active profile;
active local and replica membership counts; pending, ready, and stale vectors;
promotion cursor and pending outbox count; replica cursor and display-only
`replicatedAsOf`; and SQLite health. Context is tombstone-aware lexical context
from local personal records only. Semantic context ranking is not enabled yet,
so no cross-profile fallback can occur.

Vectors remain local and absent from `SyncBundleV1`. This pass does not enable
semantic ranking. Installation, boot behavior, timer operation, and sustained
runtime remain unverified until the reviewed artifacts are live-applied.
