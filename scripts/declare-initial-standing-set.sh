#!/usr/bin/env bash
# Declare the initial standing set: seven process rules, minted then promoted.
#
# ONE-TIME, AND A HUMAN RUNS IT. On the live store this needs an interactive
# password: /usr/local/bin/neural-memory-admin has no sudoers entry and cannot
# traverse /srv/neural-memory-data. That is not an obstacle to route around --
# it is the design ("minting and promoting are two acts, both human") enforced
# by the filesystem before the code gets a say.
#
# Kept in the repo rather than run from a shell history so WHAT was declared is
# reviewable in a diff, which is the same argument as the register pattern: a
# rule that binds every future session should not enter without a reviewable
# record of its text and its evidence.
#
# The set was resolved one rule at a time, not as a batch -- five cite a
# resolvable address, two declare that they cannot. That split is the honest
# outcome; a set where all seven cited would be the one to distrust.
#
# Claim texts carry NO addresses; every pin lives in --locator under one scheme:
#   neural-memory:<full 64-hex>   a store record
#   biofilms:<git-sha|path#row>   a repo artifact or ledger row
#   memory:<slug>.md              the file memory
#   unverified:<class>            declared, no address -- a VALUE, not an absence
#
# Usage:  bash mint_seven.sh <db-path> <iso-instant>
set -euo pipefail

DB="${1:?usage: mint_seven.sh <db> <at>}"
AT="${2:?usage: mint_seven.sh <db> <at>}"
ADMIN="${ADMIN:-neural-memory-admin}"

declare_one() {
  local claim="$1" locator="$2"
  local out digest
  out="$("$ADMIN" record-decision --db "$DB" --claim "$claim" --locator "$locator" --at "$AT")"
  echo "  $out"
  digest="${out##*digest=}"
  "$ADMIN" declare-standing --db "$DB" --digest "$digest" --at "$AT" | sed 's/^/  /'
}

echo "== 1/7 verify-in-one-snapshot"
declare_one \
"Verify by content in a single atomic snapshot immediately before mutating. Three multi-command git checks against a concurrently-committed repository each returned at least one wrong answer in one session; none caused damage, because each fired before the destructive step and the state was re-read rather than acted on." \
"neural-memory:670cb54cac52538b2dc2edf5f7ed1ad4964a67a544efee19c7a0fe9cc1851109; memory:verify-in-one-snapshot.md"

echo "== 2/7 pipes-mask-exit-status"
declare_one \
"Use set -o pipefail, and never read a PIPESTATUS index. A pipeline reports its last command's status, so a build piped to tail was read as succeeding when it had failed, and the failure was already reported as success before anyone looked." \
"unverified:session-recollection; memory:pipes-mask-exit-status.md"

echo "== 3/7 write-mode-truncates-before-read"
declare_one \
"Never open a file for writing on the same line that reads it. open(f,'w') truncates before the right-hand side runs, so open(f,'w').write(open(f).read()) empties the file, and a confirming grep printing nothing is the only signal that it happened." \
"unverified:session-recollection; memory:write-mode-truncates-before-read.md"

echo "== 4/7 no-test-data-in-persistent-stores"
declare_one \
"Never write invented or placeholder content to a persistent store. Three invented names in a taxonomy circulated in planning appeared nowhere in the source it described, and the cost was permanent: the remedy is a pinned fixture keyed by upstream commit and the sha256 of the file, with a drift script that fails rather than skips when the checkout is absent." \
"neural-memory:9a56fa26d29d97c312e8052ee528a824f07b64d6d4ae99d995c714333daeab61; memory:no-test-data-in-persistent-stores.md"

echo "== 5/7 agent-sessions-use-worktrees"
declare_one \
"Start agent sessions in a git worktree, never a shared checkout. The branch can change mid-session so a commit lands wherever HEAD points, and git push then reports Everything up-to-date as the only signal. Read branch and commit in one invocation: git status --porcelain=v2 --branch." \
"biofilms:0af4505; biofilms:f49fe94; biofilms:63be89b; memory:agent-sessions-use-worktrees.md"

echo "== 6/7 absence-claims-state-their-scope"
declare_one \
"State the scope of every absence claim, and no wider than something checks. A claim that a document had never existed rested on a grep of one branch; it was on an unmerged branch, and the false claim reached a store, was filed as a contradiction against a record that was correct, and could not be withdrawn." \
"neural-memory:9a56fa26d29d97c312e8052ee528a824f07b64d6d4ae99d995c714333daeab61; biofilms:data/claims_ledger.csv#PP-31-01; memory:absence-claims-state-their-scope.md"

echo "== 7/7 authorship-is-not-review"
declare_one \
"Do not treat your own review of your own constraint as review. A rule about atomic snapshots prescribed a two-command check in its own paragraph, and a constant read off a function's domain rather than its usable range stood until external review; neither was caught by its author. Distance cannot be acquired retroactively from a rule you already wrote." \
"biofilms:63be89b; biofilms:coupling/tests/test_pilot_seeding.py#MIN_OUTER_DRAWS; unverified:no-memory-file-yet"

echo
echo "== in force =="
"$ADMIN" standing --db "$DB"
