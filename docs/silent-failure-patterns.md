# Silent-failure patterns

Field notes from repairing a data pipeline that reported success for eight days while producing
nothing usable. Every item here was reproduced on real systems, not reasoned about. Names, hosts,
addresses and account details are omitted; the patterns are the point.

The common shape: **a check that cannot fail, a status that cannot distinguish, or an error that
was thrown away.** None of these are exotic bugs. All of them survived review because the system
kept saying it was fine.

---

## 1. A sandbox can forbid a mode you hardcoded

A daily job ended with `os.chmod(dst, 0o2775)`. Its systemd unit set `RestrictSUIDSGID=yes`, which
denies any `chmod` carrying setuid/setgid with `EPERM`. The job died there — after copying its
data, before writing its manifest — so every output directory held data files and nothing else.

The mode was hardcoded because one host genuinely needed group inheritance. It turned out **no**
host could set it: both units had the same restriction. The "it works on the other machine"
assumption had never been tested on the other machine.

```
# under a transient unit with RestrictSUIDSGID=yes
chmod 2775 <dir>   -> EPERM
chmod 0775 <dir>   -> OK
chmod 0755 <dir>   -> OK
```

**Rule.** A permission mode is host configuration, not a constant. Pass it in, restrict it to
reviewed values, and let the caller that knows the sandbox choose. A default a sandbox forbids is
a trap that fires only in production.

The setgid bit was also unnecessary: the directory's *group* was already inherited at `mkdir` from
a setgid parent, and only `g+w` was ever missing. Check what you actually need before reaching for
a special bit.

## 2. `chmod 0775` on a directory can return success and change nothing

GNU `chmod` **preserves** setuid/setgid on *directories* when given a numeric mode that omits them.

```
$ stat -c %a d      # 2775
$ chmod 0775 d; echo $?   # 0
$ stat -c %a d      # 2775   <- unchanged
$ chmod g-s d
$ stat -c %a d      # 775
```

Python's `os.chmod` is the raw syscall and is *not* affected, so code and shell disagree about what
the same numeric mode means. If a shell script normalises directory modes, use symbolic `g-s`.

## 3. `systemd-analyze verify` exits 0 on a broken unit

A fragment containing a bare `ThisIsNotADirective` line produces
`Missing '=', ignoring line.` on stderr — and **exit status 0**. The status carries no
information; only the output does.

But the output is also unusable raw: it loads the entire dependency closure, so it reports on
unrelated distro units, and it mixes genuine parse errors with advisories like
`RuntimeMaxSec= has no effect in combination with Type=oneshot`.

A first attempt treated any unexpected line as fatal. That is the better instinct in the abstract
and it produced two deploy-blocking false positives in a row. **A deploy tool that cries wolf gets
bypassed, which fails less safely than a narrower rule.** Final shape: scope output to lines naming
the fragment under test, match fatal patterns explicitly, and *print* everything else rather than
hiding it.

## 4. A success-status list can swallow a parse failure

The unit declared `SuccessExitStatus=2` so that a legitimate "already done today" run counted as
success. `argparse` also exits **2** on an unrecognised option.

So a deploy that reverted the program while leaving a newer unit in place would pass a flag the old
program didn't know, exit 2, and be reported by the service manager as a **successful run that did
nothing at all** — strictly worse than the original bug, which at least left a failed unit.

**Rule.** Never share an exit code between "this succeeded in a boring way" and "the runtime
rejected my command line". Reserve a distinct code for the boring success and leave the
interpreter's own failure codes as failures.

## 5. "The output exists" is not "the work finished"

The job was write-once: if the target directory existed it returned early, reporting success. That
is how truncated outputs kept reporting success on every re-run for over a week — the check could
not distinguish *finished* from *started*.

The manifest could not help: it was written *before* the remaining files were copied.

**Rule.** Write a completion marker **last**, carrying hashes of everything the operation was
supposed to produce, and verify it on *both* paths — when re-encountering existing output *and*
immediately after creating new output. An existence check is not a completeness check.

Watch for the degenerate version of this. The first implementation iterated
`(manifest.get("files") or {})`, so an empty manifest verified clean — *nothing to check, so it
passed*, the same shape as the original defect wearing a manifest.

## 6. Conditional copies produce silent incompleteness

Each auxiliary file was copied behind `if os.path.exists(src)`. A missing source was skipped in
silence and the run still exited 0.

**Rule.** Preflight the *sources* before creating anything, so a missing input fails immediately
and leaves no half-built output for a later run to mistake for finished work. Copy to a temp name,
validate the content parses, then `os.replace()` — presence is not integrity, and a truncated file
exists.

## 7. `After=` means "after it finished", not "after it succeeded"

A follow-on unit was pulled in with `Wants=` and ordered with `After=`. Service managers draw no
distinction between finished and succeeded from ordering alone, so a failed run was still followed
by its downstream step, publishing the incomplete result as consumable.

`OnSuccess=` is the dependency that actually encodes the intent. Enforce it in the program too —
the chain edge and the program's own refusal should both hold, so neither is the only guard.

## 8. Discarded stderr converts a one-command diagnosis into hours

A shell function ended its call with `2>/dev/null`. When the underlying transport died, the caller
logged only `value=''` — true, useless, and repeated every cycle for hours. Allowing stderr through
identified the failure in a single command: a connection refused to a proxy that was no longer
listening.

The fail-closed guard around it was correct and must not be weakened. Only the discarded reason was
the problem. **Fail closed, but never fail silent about why.**

## 9. A crash-looping service reports `active (running)`

The dead transport was a unit restarting every ten seconds or so. Between restarts it is genuinely
active, so `systemctl is-active` returns `active` and a casual health check passes. `NRestarts` was
in the four figures.

Check restart counters, not just current state.

## 10. Absence in one place is not absence

Two false conclusions in one investigation, both from checking a single location:

- "The upstream repository does not exist" — it did; the code lived on a branch nobody had looked
  at. The default branch had been searched, and the deployed commit resolved in none of the local
  clones *of that branch*.
- "These positions have no protective stop orders" — they all had one. The API exposes stop and
  trigger orders on a **different endpoint** from ordinary orders; the first endpoint honestly
  returned zero.

**Rule.** State the scope with the claim: "not in the default branch", "not in the orders
endpoint". An unscoped absence claim is a guess wearing a fact's clothes.

## 11. A negative control that cannot reproduce the defect proves nothing

Testing that an interrupted copy is caught, the obvious control is to make the source unreadable.
That fails at `open()`, leaves **no** destination behind, and never exercises the code path. The
control has to write a *partial destination* and then fail.

Related: mutation-test every guard. One guard here survived the whole suite, because every caller
pre-checked before reaching it — defence in depth with no test on the depth. It got its own test
only after a mutation revealed the gap.

## 12. Config-syncing tools must back up what they overwrite

A tool that reconciles deployed configuration against a repository is doing exactly its job when it
reverts local drift. But on its first run it replaced a file that differed from the repository, in a
directory not under version control, with no copy kept — so the drift's *intent* is now
unrecoverable, and nobody can say whether it mattered.

**Rule.** Reconciling to a source of truth is fine. Being the only copy of what you replaced is
not. Back up first, and treat the lost content as an open question rather than assuming the
repository version was what the operator meant.

---

## The shared lesson

Every one of these reports success. That is what makes them expensive: the monitoring, the exit
status and the log line all agree, and the only thing that disagrees is the artifact nobody
re-opened. When a check passes, ask what it would take for it to fail — and if you cannot answer,
you do not yet have a check.
