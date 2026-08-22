# Personal memory banking policy

Policy identifier: `PersonalMemoryBankingPolicyV1`

This policy belongs to the stores, not to a particular agent. Codex, Claude,
and any future client must follow the same routing rules.

## Store roles

- The Mac `claude-mind` Core Data store is canonical for personal memory.
- The GPD `personal.db` accepts local captures into a promotion outbox and keeps
  a read-only canonical replica for offline recall.
- The GPD neural-memory evidence `store.db` remains canonical for research
  evidence. It is never merged into personal memory. Disaster recovery uses
  verified, signed `VACUUM INTO` backups instead.
- Clients read only their local store. There is no live remote recall, remote
  SQL, remote filesystem access, or remote MCP proxying.

## What may be banked

At establishment, bank durable user facts, decisions, preferences, project
constraints, and explicitly requested reminders. A capture should be useful
beyond the current turn and should include enough context to remain
intelligible. An explicit `/remember` request retains eligible content even if
the client would not have auto-banked it.

Bank a correction or contradiction immediately. Record the corrected durable
fact and tombstone or supersede the contradicted personal record as one logical
operation; do not silently overwrite history or resolve divergence by
last-writer-wins.

When two canonical records cannot yet be resolved, store an explicit unordered
divergence pair. Both records remain durable, but recall and context withhold
both while the pair is unacknowledged. The unresolved pair is listable through
the local interface. Acknowledgement restores both records without selecting a
winner; any later correction remains an explicit tombstone or supersession.

Route only structurally valid research evidence to the local
`neural-memory-server` evidence MCP. Evidence is not personal memory and never
enters personal recall.

Never auto-bank:

- passwords, API tokens, session cookies, private keys, recovery codes, or raw
  authentication material;
- full payment-card, bank-account, government-ID, or tax identifiers;
- raw biometric, EEG, audio, or medical records;
- private third-party content that the user did not ask to retain;
- transient command output, chain-of-thought, hidden prompts, or model scratch
  work;
- speculation, unverified guesses, raw logs, and other ephemeral material;
- research evidence records from neural-memory `store.db`.

Explicit user direction may permit a durable summary of sensitive material,
but never secret-bearing raw values. `/remember` does not override the secret,
credential, raw-record, or third-party privacy exclusions.

## Identity and convergence

- Identity domain: `claude-mind.memory.v1`.
- Digest input is the RFC 8785 canonical JSON object with keys
  `contentDomain`, `metadata`, `occurredAt`, and `text`.
- Text is NFC-normalized. `occurredAt` is null or UTC RFC 3339 with exactly
  milliseconds. Non-finite metadata numbers are rejected.
- Metadata object keys are validated after JSON escape decoding and before any
  language dictionary is constructed. At every nesting level, keys must
  already be NFC; exact duplicates, non-NFC keys, normalization collisions,
  and invalid Unicode are rejected. RFC 8785 then runs without normalizing
  keys. This narrows JCS input to avoid Swift/Rust map-key divergence.
- Capture time, source, conversation, origin, tags, and sightings are excluded
  from identity. Equal durable content therefore converges across sessions.
- On digest match, retain `min(createdAt)`, union tags, and retain each distinct
  `(originDevice, originRecordID)` as a sighting.
- Textually identical, undated saves intentionally become one canonical record.
- Fetch-or-insert is transactional. The Core Data model has no uniqueness
  constraint, preserving the option to use `NSPersistentCloudKitContainer`.

Golden vectors are in `identity-v1-vectors.json` and
`ed25519-v1-vectors.json` in this directory. Both implementations load these
published files; neither test embeds a substitute vector.

## Sync and trust

- `SyncBundleV1` uses an `(epoch, sequence)` cursor. Display timestamps never
  decide freshness.
- The Ed25519 signature covers the literal decoded `payloadBase64` bytes.
  Verify before JSON parsing, then recompute every content digest before import.
- A receiver acknowledges `through` only after the entire import and peer cursor
  commit atomically. A forward bundle must start at the exact committed cursor.
  The latest transition is bound to its change-history fingerprint, so an exact
  replay is a no-op while same-cursor equivocation, cursor gaps, epoch skips,
  rollback, and origin-ID reuse fail closed.
- A bundle carries the complete divergence set only when `through` reaches the
  sender's current record head. Intermediate chunks and old-epoch tails carry
  none, preventing references to records not yet received. At the head,
  divergence state is merged in the record transaction and may advance without
  advancing the record cursor. Acknowledgement is monotonic; replaying an older
  unacknowledged state cannot reopen an acknowledged pair.
- Mac signing keys live in Keychain with
  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`. GPD signing keys live inside
  the encrypted personal-memory mount.
- Key compromise recovery is an explicit stopped-transport procedure: revoke
  the old key from local coordinator trust, rotate the compromised key, bump
  the source epoch while emitting every canonical record as a sequence-one
  fresh snapshot, reset only replica-derived state on the receiving GPD when
  required, enroll the replacement peer key from a local admin surface, and
  resume only after the new key ID and snapshot cursor are verified. Old epoch
  counters are not reused. An embedded or request-supplied public key is never
  trusted. GPD replica reset preserves local captures and promotions, removes
  only replica-derived state, and pins an explicit predecessor epoch. Rotation,
  reset, enrollment, and fresh-snapshot commands are confirmation-gated local
  operations.
- The Mac initiates synchronization. Tailnet-restricted forced-command SSH is
  primary; tailnet HTTPS provides the same restricted operations as fallback:
  status, ordered export, acknowledgement, and verified import. Rotation,
  enrollment, replica reset, and epoch bump are never remote operations.
- The coordinator preflights the data-free remote `status` operation and
  atomically publishes `sync_status.json`. A locked GPD data volume is
  `blocked-on-mount`, exits successfully without export or import, and is not
  reported as a protocol failure. Unreachable transport and malformed status
  responses remain distinct failures.

## Evidence backups

Evidence routing decision: `EvidenceRoutingDecisionV1`.

Evidence remains canonical only in the GPD `neural-memory-server` store under
its existing schema and `nms.record.v1` identity. The personal coordinator MUST
reject that domain and MUST NOT promote, merge, or expose evidence through
personal recall. The Mac receives evidence only as an opaque, signed, verified
SQLite disaster-recovery artifact. Retaining that artifact does not make the
Mac an evidence writer or evidence recall server.

Evidence recovery is backup, not synchronization. The GPD creates a consistent
`VACUUM INTO` backup, verifies all evidence digests/observations/provenance with
the existing verifier, signs its manifest, and only then permits the Mac to
pull it. Personal tables never enter `store.db` or its verifier surface.

The Mac accepts a pulled artifact only after `claude-mind-sync
verify-evidence-backup` verifies the enrolled GPD Ed25519 signature over the
literal manifest bytes, the fixed filename, verifier/count attestation, byte
length, SHA-256, and local SQLite `integrity_check`. Retention is atomic and
does not import evidence rows into the personal-memory store.
