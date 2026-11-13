# ADR: Use the GPD's Local Memory Replica for Recall

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The Mac is canonical for personal memory. The GPD keeps a read-only canonical
replica in its local `personal.db` for recall. The GPD can search that local
copy whether or not the Mac is reachable. The copy reflects only the most
recently completed synchronization; it does not establish that the Mac has no
newer records.

Replica freshness and storage availability are separate. The encrypted data
mount holds `personal.db` and the replica. If it is not mounted, the store is
unavailable regardless of the last sync time. Automatic TPM unlock has not
been confirmed for this setup, so a reboot can require operator unlock and
mount before local recall is available.

The agent session uses two deployment-level aliases:

- `sync_status` reports the synchronization state available to the GPD.
- `recall_personal` searches the GPD's local personal-memory store.

These aliases are provided by the deployment, not by this repository's MCP
tool names. The repository exposes personal MCP `recall`. The coordinator's
`sync_status.json` is documented in [`BANKING_POLICY.md`](../BANKING_POLICY.md);
the display-only `replicatedAsOf` field is documented in
[`personal-memory.md`](../personal-memory.md).

## Decision

For personal-memory recall from the GPD, inspect `sync_status` first, then call
`recall_personal`. The recall runs against the GPD's local replica and does not
query the Mac for memory content. The deployment labels hits from imported Mac
canonical records as `canonicalReplica`; this label identifies their replica
origin and does not prove that they are the latest records on the Mac.

If the encrypted data mount is absent, `sync_status` reports
`health: blocked-on-mount`. `recall_personal` fails closed with reason
`blocked-on-mount` and states that the encrypted memory volume is unavailable.
It must not use another database, contact the Mac for recall, or route the
request to a cloud service. The data-free status path must not open the
database or signing key to report this condition.

Report freshness only from the available sync status and last completed sync.
If status or sync time is unavailable, describe freshness as unknown. Do not
infer currentness from the Mac's network reachability or from a successful
local recall.

The recall path is read-only. It does not fetch PDFs, database files, or other
files from the Mac, and it does not expose SQL, shell, filesystem, or remote
MCP access. A PDF can match only if its text is already present in an eligible
personal-memory record. Recall results remain personal data and must not be
forwarded to cloud generators or automatically saved as new memory records.
Any separate explicit request to remember information remains governed by
`PersonalMemoryBankingPolicyV1`.

This decision follows [`BANKING_POLICY.md`](../BANKING_POLICY.md): clients read
their local personal store, and live remote recall is not available. It does
not add a remote query service or change the sync protocol.

## Consequences

- A GPD agent can search the replica while the Mac is offline or unreachable.
- Recall is unavailable while the encrypted data volume is not mounted; the
  last known replica does not make a locked store readable.
- Results may be stale; callers must preserve the status-derived freshness
  limits when they report or use them.
- Access to original files remains a separate, explicitly approved workflow;
  this recall path does not provide it.
- `sync_status` and `recall_personal` must remain clearly identified as
  deployment aliases so they are not mistaken for repository-native tools.
