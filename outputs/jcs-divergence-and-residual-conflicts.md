# JCS Cross-Implementation Divergence and Residual Conflict Classes

**Scope:** Two narrow questions for a two-device, content-digest-identity memory system
(Swift/Core Data canonical + Rust/SQLite local-first). Option space and documented
trade-offs only. **No design is recommended anywhere in this document.**

**Date:** 2026-08-06
**Status:** Part 1 partly measured on this machine, partly documented-only. Part 2 documented-only.

---

## Evidence grading used throughout

| Tag | Meaning |
|---|---|
| `[MEASURED]` | I compiled and ran it here. Raw output: `experiments/jcs-divergence/results.txt` |
| `[SOURCE]` | I read the implementation source directly |
| `[SPEC]` | I read the specification text directly |
| `[ISSUE]` | I read the issue/PR thread directly |
| `[PAPER]` | Paper existence + title/abstract verified at the identifier given |
| `[2ND]` | From a delegated researcher's notes; **I did not independently verify.** Treat as a lead |
| `[INFERRED]` | Follows logically from something I read, but was not itself observed |

**Every `[2ND]` and every single-source claim is flagged inline.** Where a divergence class
turned out to have *no* reported disagreement, that is stated explicitly, as requested.

---

## 0. Two premise corrections before the findings

These change the shape of both questions, so they come first.

### 0.1 JCS does not apply NFC to keys **or** values — it forbids normalizing either

Your question asked "whether NFC is applied to keys as well as values." The answer is
**neither, and applying it inside a JCS implementation would be non-conformant.**

RFC 8785 §3.1 `[SPEC]`:

> Note: Although the Unicode standard offers the possibility of rearranging certain
> character sequences, referred to as "Unicode Normalization" [UCNORM], JCS-compliant
> string processing does not take this into consideration. That is, all components
> involved in a scheme depending on JCS **MUST preserve Unicode string data "as is"**.

And §3.1 again, on the same point:

> An additional constraint is that parsed JSON string data MUST NOT be altered during
> subsequent serializations.

So NFC is an **application-layer** step in your pipeline, strictly upstream of
canonicalization. That is not itself a violation — but it means the *ordering* of your two
steps (NFC-then-JCS) is load-bearing and must be byte-identical on both devices, and JCS
gives you no help enforcing it. See §2.1 for what breaks.

Source: <https://www.rfc-editor.org/rfc/rfc8785#section-3.1>

### 0.2 Your "key ordering when keys differ only after NFC" case is not a JCS question

Because of 0.1, JCS sorts the **raw, pre-normalization** UTF-16 code units. Two keys that
are NFC-equivalent but differently encoded are, to JCS, simply two different keys, and both
survive. `[SPEC]` §3.2.3:

> The sorting process is applied to property name strings in their "raw" (unescaped) form.

`[MEASURED]` — all four Rust crates keep both keys and sort them by code unit:

```
=== NFC-equivalent distinct keys  U+00E9 vs U+0065 U+0301
  input             : {"é":1,"é":2}
  serde_jcs 0.1.0   : {"é":2,"é":1}      <- decomposed (0x65) first, precomposed second
  serde_jcs 0.2.0   : {"é":2,"é":1}
  json-canon 0.1.3  : {"é":2,"é":1}
  sjc 0.3.x         : {"é":2,"é":1}
```

**No divergence among Rust implementations on this class.** The divergence is Swift-vs-Rust,
and it is not in the canonicalizer — it is in the language's `String` type. See §1.5.

---

# PART 1 — RFC 8785 / JCS divergence in practice

## 1.0 What I actually measured

I built a probe against four Rust JCS crates at pinned versions and ran the RFC's own
conformance vectors plus edge cases. Reproduce with:

```
cd experiments/jcs-divergence && cargo run
```

Toolchain: rustc 1.97.1. Crates resolved: `serde_jcs 0.1.0`, `serde_jcs 0.2.0`,
`json-canon 0.1.3`, `serde_json_canonicalizer 0.3.x`. Raw output committed to
`experiments/jcs-divergence/results.txt`.

**No Swift toolchain is available on this machine.** Every Swift claim below is `[SOURCE]`
(I read swift-foundation source) or `[2ND]`/`[INFERRED]`, never `[MEASURED]`. This is the
single biggest gap in this report.

---

## 1.1 Number serialization

### 1.1.1 Negative zero — **NO DIVERGENCE FOUND** (Rust side)

RFC 8785 Appendix B `[SPEC]` requires IEEE `8000000000000000` → `0`.

`[MEASURED]` all four crates emit `0`, both when parsed from `-0.0` and when fed
`-0.0f64` directly:

```
=== -0.0f64 direct  (f64 bits = 8000000000000000)
  serde_jcs 0.1.0   : 0
  serde_jcs 0.2.0   : 0
  json-canon 0.1.3  : 0
  sjc 0.3.x         : 0
  rust f64 Display  : -0        <- note: Rust's own Display is WRONG for JCS
```

The last line matters: Rust's native `f64` Display emits `-0`. Every crate correctly
overrides it. Any hand-rolled canonicalizer that uses `format!("{}", x)` gets this wrong.

**Swift side: divergence is likely, `[SOURCE]` + `[INFERRED]`.** swift-foundation's
JSONEncoder does:

```swift
var string = float.description
if string.hasSuffix(".0") {
    string.removeLast(2)
}
return .number(string)
```
<https://github.com/swiftlang/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONEncoder.swift>

Swift's `(-0.0).description` is `"-0.0"`; stripping `".0"` yields **`-0`**, not `0`.
I read the stripping code `[SOURCE]`; I did **not** measure `(-0.0).description`
`[INFERRED]`. **This needs a one-line measurement on a Mac before you rely on it.**
Note swift-foundation's JSONEncoder is *not* a JCS implementation — it is the substrate a
Swift implementer would most likely build on.

### 1.1.2 Integers at the 2^53 boundary — **MAJOR DIVERGENCE, four-way split**

This is the most consequential number finding. `[MEASURED]`:

| Input | serde_jcs 0.1.0 | serde_jcs 0.2.0 | json-canon 0.1.3 | sjc 0.3.x |
|---|---|---|---|---|
| `9007199254740992` (2^53) | `9007199254740992` | `9007199254740992` | **ERROR** | `9007199254740992` |
| `9007199254740993` (2^53+1) | `9007199254740993` | `9007199254740992` | **ERROR** | `9007199254740992` |
| `18446744073709551615` (u64 max) | `18446744073709551615` | `18446744073709552000` | **ERROR** | `18446744073709552000` |
| `-9223372036854775808` (i64 min) | `-9223372036854775808` | `-9223372036854776000` | ***PANIC*** | `-9223372036854776000` |

Three distinct behaviours for the same input:

- **serde_jcs 0.1.0** passes large integers through **exactly**, never round-tripping via
  `f64`. This is a silent conformance break: RFC 8785 §3.1 `[SPEC]` requires
  "JSON number data MUST be expressible as IEEE 754 double-precision values."
  0.1.0 emits digit strings no ECMAScript serializer would ever produce.
- **serde_jcs 0.2.0 / sjc** round through `f64` — matches the ECMAScript reference.
- **json-canon 0.1.3** rejects with `u64 must be less than JSON max safe integer` —
  and rejects **at exactly 2^53**, which RFC 8785 Appendix B lists as a *valid* value
  (`4340000000000000 | 9007199254740992 | Max pos int`). This is an off-by-one against
  the RFC's own table.

**Bearing on your design:** you reject non-finite numbers but the RFC's guidance
`[SPEC]` Appendix B note (1) is about *integers*: values meant as true integers
"SHOULD be in the range -9007199254740991 to 9007199254740991." Anything in your content
metadata that can exceed 2^53 (byte offsets, nanosecond timestamps, IDs) is a live
cross-implementation digest divergence. Appendix D's stated remedy is to carry such
numbers as JSON strings.

### 1.1.3 `json-canon 0.1.3` panics on `i64::MIN` — **new, previously unreported**

`[MEASURED]`. This is a crash, not a divergence:

```
thread 'main' panicked at library/core/src/num/mod.rs:450:5:
attempt to negate with overflow
```

Triggered by `{"x":-9223372036854775808}`. In a replication path that parses
attacker- or peer-supplied JSON, this is a remote panic on a one-byte-cheap input.
I found **no issue filed** for this. Single-source: **this measurement is the only
evidence; it is mine, and it has not been confirmed by the maintainer.**

### 1.1.4 Values that overflow to infinity — **divergence is at the PARSER, not the canonicalizer**

`[MEASURED]`:

```
=== overflow to infinity (1e400)
  serde_json PARSE  : ERROR: number out of range at line 1 column 10
```

`serde_json` **rejects `1e400` at parse time**, so it never reaches any canonicalizer.
ECMAScript, by contrast, parses `1e400` to `Infinity`. So the divergence is:
Rust = hard parse error; JS/Swift-via-Double = `Infinity`, which JCS then requires be an
error. Both terminate, but with different errors at different layers — meaning your two
devices will produce **different failure modes and different error messages** for the same
bundle, and any code that distinguishes "malformed" from "out of range" will disagree.
`[INFERRED]` for the Swift half — not measured.

### 1.1.5 Non-finite values — **DIVERGENCE, and it defeats a stated invariant of yours**

You state "non-finite numbers rejected." Two of four crates **do not reject them**.
`[MEASURED]`:

```
=== f64::NAN  (f64 bits = 7ff8000000000000)
  serde_jcs 0.1.0   : null           <- silently emits null
  serde_jcs 0.2.0   : ERROR: invalid float value
  json-canon 0.1.3  : null           <- silently emits null
  sjc 0.3.x         : ERROR: NaN and +/-Infinity are not permitted in JSON
```

Identical results for `+Infinity` and `-Infinity`.

RFC 8785 §3.2.2.3 `[SPEC]`:

> Note: Since Not a Number (NaN) and Infinity are not permitted in JSON, occurrences of
> NaN or Infinity **MUST cause a compliant JCS implementation to terminate with an
> appropriate error.**

`serde_jcs 0.1.0` and `json-canon 0.1.3` are **non-conformant** here. `null` is not a
rejection — it is a value that will hash, and it collides with a genuine `null`. Two
records, one with `NaN` and one with `null`, get the **same digest**.

This is reachable only via programmatic construction (`f64::NAN`), not via parsing, since
JSON text cannot express NaN. Given your digest is computed over programmatically built
metadata, that is exactly your path. My delegated researcher independently reported this
class for `json-canon` and `canon-json` `[2ND]`; I measured it for `json-canon` and
`serde_jcs 0.1.0`. **The `canon-json` half is `[2ND]` and unverified by me.**

### 1.1.6 Exponent formatting — **NO DIVERGENCE FOUND** (Rust side)

`[MEASURED]` — all four crates agree, and all match RFC 8785 Appendix B exactly:

| Input | All four crates | RFC 8785 App. B |
|---|---|---|
| `1e21` | `1e+21` | `1e+21` ✅ |
| `1e-7` | `1e-7` | `9.999999999999997e-7` family ✅ |
| `5e-324` | `5e-324` | `5e-324` ✅ |
| `295147905179352830000` (2^68) | `295147905179352830000` | `295147905179352830000` ✅ |
| `1424953923781206.25` | `1424953923781206.2` | `1424953923781206.2` (round-to-even) ✅ |
| RFC §3.2.2 worked example | `[333333333.3333333,1e+30,4.5,0.002,1e-27]` | identical ✅ |

**This is a clean absence.** All four Rust crates delegate to `ryu-js` (or equivalent) and
reproduce ECMAScript `Number::toString` including the `1e+21` / `1e-7` thresholds, the
`e+`/`e-` sign, and no zero-padding of the exponent.

**Swift is the risk, and it is documented-only `[2ND]`/`[INFERRED]`.** My researcher
reports Swift's `Double.description` switches to exponential above **2^53** (not 1e21) and
below **1e-4** (not 1e-6), and zero-pads exponents to two digits (`1e-07` vs `1e-7`).
I verified the surrounding swift-foundation code `[SOURCE]` but **not these thresholds**.
Three RFC Appendix B vectors would fail structurally if true. **This is the highest-value
thing to measure on a Mac and I could not do it here.**

---

## 1.2 Key ordering — UTF-16 code units vs everything else

RFC 8785 §3.2.3 `[SPEC]` mandates UTF-16 code-unit order and explicitly warns:

> Note: For the purpose of obtaining a deterministic property order, sorting of data
> encoded in UTF-8 or UTF-32 would also work, but the outcome for JSON data like above
> would differ and thus be incompatible with this specification. **However, in practice,
> property names are rarely defined outside of 7-bit ASCII**, making it possible to sort
> string data in UTF-8 or UTF-32 format without conversion to UTF-16 and still be
> compatible with JCS.

That last sentence is the practical scoping: **UTF-8 and UTF-16 order coincide for all of
the BMP below U+E000.** They diverge only when a key contains a non-BMP character
(U+10000+, which encodes as surrogates D800–DBFF) competing against a key starting
U+E000–U+FFFF.

### 1.2.1 `serde_jcs 0.1.0` fails the RFC's own conformance vector — **CONFIRMED, reported**

`[MEASURED]`, on RFC 8785 §3.2.3's verbatim test object:

```
  serde_jcs 0.1.0   : {"1","\r","\u0080","ö","€","דּ","😀"}   <- WRONG (two ways)
  serde_jcs 0.2.0   : {"\r","1","\u0080","ö","€","😀","דּ"}   <- matches RFC
  json-canon 0.1.3  : {"\r","1","\u0080","ö","€","😀","דּ"}   <- matches RFC
  sjc 0.3.x         : {"\r","1","\u0080","ö","€","😀","דּ"}   <- matches RFC
```

RFC expected order: Carriage Return, One, Control, ö, Euro, **Emoji, Hebrew U+FB33**.
0.1.0 gets **both** the `\r`/`1` pair and the emoji/U+FB33 pair wrong.

**Reported at:** l1h3r/serde_jcs issue #1, "Sorting properties as unescaped UTF-16",
filed by clehner, **open** `[ISSUE]` — <https://github.com/l1h3r/serde_jcs/issues/1>

I read the thread. The reporter's diagnosis, verbatim:

> I think our sorting comes from `BTreeMap`. But the key type is `Vec<u8>`. So I think the
> properties are being sorted in UTF-8, and maybe in their escaped form - I'm not sure.

He cross-checked against the npm `json-canonicalize` module and confirmed his expected
output came from an independent implementation.

**Correction to a claim I was given.** My delegated researcher framed this issue as
demonstrating the UTF-8-vs-UTF-16 divergence. Reading the actual thread, the vector the
reporter used (`\n`, `</script>`, `1`, …) demonstrates the **escaped-form** bug —
`\n` serialized as `\` `n` = 0x5C 0x6E sorts *after* `<` = 0x3C. `[MEASURED]` confirms
0.1.0 exhibits **both** defects independently:

```
=== escaped-vs-raw sorting (serde_jcs issue #1)
  serde_jcs 0.1.0   : {"1","</script>","\n"}     <- escaped-form order
  others            : {"\n","1","</script>"}     <- raw-form order, correct

=== non-BMP vs high-BMP  (UTF-16 vs scalar order)
  input             : {"\uffa5":1,"\ud842\udfb7":2}
  serde_jcs 0.1.0   : {"ﾥ":1,"𠮷":2}             <- scalar/UTF-8 order
  others            : {"𠮷":2,"ﾥ":1}             <- UTF-16 order, correct
```

### 1.2.2 Version-skew exposure

`serde_jcs` went `0.1.0` → `0.2.0`, a **semver-incompatible** bump. A dependent pinned
`serde_jcs = "0.1"` never receives the fix. My researcher reports 0.1.0 still drew
~570k downloads in a recent 90-day window against ~751k for 0.2.0 `[2ND]` — **I did not
verify these download figures.** The qualitative point stands on my own measurement: two
versions of the same crate name produce different canonical bytes for the same input.

### 1.2.3 Swift `sortedKeys` sorts by UTF-8 — **CONFIRMED FROM SOURCE**

`[SOURCE]` — swift-foundation `JSONWriter.swift`, lines 297–301, which I fetched and read:

```swift
// If we didn't use the NSString-based compatibility sorting, sort lexicographically by the UTF-8 view
if !compatibilitySorted {
    let elems = dict.sorted { a, b in
        a.key.utf8.lexicographicallyPrecedes(b.key.utf8)
    }
```

**UTF-8 byte order, not UTF-16 code-unit order.** Per §1.2 this is JCS-incompatible exactly
when non-BMP keys are present.

Worse, there is a second path in the same function `[SOURCE]`, lines 274–291:

```swift
if JSONEncoder.compatibility1 {
    // If applicable, use the old NSString-based sorting with appropriate options
    compatibilitySorted = true
    ...
    let options: String.CompareOptions = [.numeric, .caseInsensitive, .forcedOrdering]
    let locale = Locale.system
    return a.key.compare(b.key as String, options: options, range: range, locale: locale) == .orderedAscending
```

This is **numeric-aware, case-insensitive, and locale-dependent**. It is not a canonical
order under any definition — `Locale.system` alone makes the output machine-dependent.
Gated on `FOUNDATION_FRAMEWORK` + a `compatibility1` flag, so it is the Apple-platform
legacy path, i.e. the one a macOS canonical store is most likely to hit.

Source: <https://github.com/swiftlang/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONWriter.swift>

**Caveat, stated plainly:** `JSONEncoder.sortedKeys` is not claimed by Apple to be JCS.
This is evidence that the obvious Swift substrate is unsuitable, not evidence that a Swift
JCS library is wrong.

---

## 1.3 Lone surrogates and invalid UTF-16 — **NO DIVERGENCE FOUND (Rust side)**

RFC 8785 §3.2.2.2 `[SPEC]`:

> Note: Since invalid Unicode data like "lone surrogates" (e.g., U+DEAD) may lead to
> interoperability issues including broken signatures, occurrences of such data MUST cause
> a compliant JCS implementation to terminate with an appropriate error.

`[MEASURED]` — `serde_json` rejects all three cases **at parse**, before any canonicalizer
is reached:

```
=== lone leading surrogate \ud800
  serde_json PARSE  : ERROR: unexpected end of hex escape at line 1 column 13
=== lone trailing surrogate \udead
  serde_json PARSE  : ERROR: lone leading surrogate in hex escape at line 1 column 12
=== lone surrogate in KEY
  serde_json PARSE  : ERROR: unexpected end of hex escape at line 1 column 9
```

**Clean absence on the Rust side, and it is structural, not incidental:** Rust's `String`
is guaranteed well-formed UTF-8, `char::from_u32(0xD800)` returns `None`, so a lone
surrogate is unrepresentable. No Rust JCS crate can emit one, and none can silently
replace it with U+FFFD on this path. `[MEASURED]` + `[INFERRED]` for the type-level
argument.

**The asymmetry is the finding.** Swift's `String` is also UTF-8-backed and cannot hold a
lone surrogate; but Swift's `JSONDecoder`/`JSONSerialization` behaviour on lone-surrogate
*input* — error vs U+FFFD replacement — I **did not verify** and my researcher did not
report it. **This is an open question, not a finding.** If Swift replaces with U+FFFD where
Rust errors, that is a silent digest divergence rather than a matched rejection.

---

## 1.4 Duplicate keys — **NO DIVERGENCE FOUND on the path I tested; a contradiction I had to resolve**

`[MEASURED]`, via `serde_json::Value`:

```
=== duplicate keys (I-JSON forbids)
  input             : {"a":1,"a":2,"b":9}
  serde_jcs 0.1.0   : {"a":2,"b":9}
  serde_jcs 0.2.0   : {"a":2,"b":9}
  json-canon 0.1.3  : {"a":2,"b":9}
  sjc 0.3.x         : {"a":2,"b":9}
```

All four agree: **last-wins**, silently. None errors, though RFC 8785 §3.1 `[SPEC]` says
"JSON objects MUST NOT exhibit duplicate property names."

**Explicit contradiction with my delegated researcher, resolved in favour of measurement.**
The researcher reported a "four-way split on duplicate keys" (serde_jcs last-wins, sjc
first-wins, json-canon emits both, vr-jcs rejects) `[2ND]`. My measurement shows no split.
The reconciliation: the researcher's claim was scoped to the **non-`Value` `serialize_map`
path** (serializing a Rust struct/map type directly), whereas I tested the `Value` path,
where `serde_json::Map` collapses duplicates at *parse* time before any canonicalizer sees
them. **Both can be true, on different paths.** I have verified only the `Value` path;
the `serialize_map` split remains `[2ND]` and unverified.

This matters for you because it determines *where* the collapse happens: if you build
metadata as a typed Rust struct and serialize directly, you are on the unverified path.

---

## 1.5 The NFC divergence that actually threatens this design

This is Swift-vs-Rust at the **language** level, and it is the most likely source of a real
digest split. It is documented, not measured.

**Swift `String` `==` and `<` use Unicode canonical equivalence.** Apple's `String`
documentation `[SPEC]`, verbatim:

> Comparing strings for equality using the equal-to operator (`==`) or a relational
> operator (like `<` or `>=`) is always performed using Unicode canonical representation.
> As a result, different representations of a string compare as being equal.

<https://developer.apple.com/documentation/swift/string>
Mirrored in stdlib source: <https://github.com/apple/swift/blob/main/stdlib/public/core/String.swift>

**Rust `String` `==` and `Ord` are byte comparisons.** No normalization, ever.

Consequences, both of which I partly measured:

**(a) Swift `Dictionary` silently merges NFC-equivalent keys; Rust keeps them separate.**
`[MEASURED]` on the Rust half (§0.2): all four crates keep `"é"` (U+00E9) and
`"e\u{301}"` as two keys. `[INFERRED]` on the Swift half: a `[String: Any]` keyed on those
two strings has **one** entry. Different key *count* → different canonical bytes →
different digest. Not a canonicalizer bug on either side; a type-system mismatch.

**(b) The Kelvin-sign case, where your own NFC step creates the collision.**
`[MEASURED]`:

```
=== Kelvin sign vs ASCII K
  input             : {"K":1,"K":2}          <- U+004B and U+212A
  all four crates   : {"K":1,"K":2}          <- two keys, preserved
```

U+212A KELVIN SIGN has a *singleton canonical decomposition* to U+004B, so
`NFC(U+212A) == U+004B`. Therefore:

- In **Swift**, these are already one key before you do anything — see Swift issue
  SR-3754, "Two Strings are considered equivalent when they are not", where exactly this
  pair is reported and closed as intended behaviour `[ISSUE]`:
  <https://github.com/apple/swift/issues/46339>
- In **Rust**, they are two keys `[MEASURED]` — until your NFC step runs, at which point
  they **become a duplicate key**, i.e. an I-JSON violation (§1.4), silently resolved
  last-wins.

So NFC-normalizing metadata *keys* converts a Swift/Rust key-count mismatch into a
Rust duplicate-key collapse. NFC-normalizing only *values* leaves the key-count mismatch
in place. **Both branches have a documented failure; the trade-off is which one.**
`[MEASURED]` for the Rust behaviour, `[SPEC]`+`[ISSUE]` for the Swift behaviour,
`[INFERRED]` for the combination.

---

## 1.6 Swift JCS implementations: the availability finding

`[2ND]`, and I flag it as **single-source and important**: my researcher reports that
**no production-grade Swift JCS implementation exists.**

- RFC 8785 Appendix G `[SPEC]` — I read it directly. It lists exactly five
  "verified to be compatible" implementations: JavaScript (`canonicalize`), Java
  (`erdtman/java-json-canonicalization`), Go, .NET/C#, Python. `grep -ic swift` over the
  full RFC returns **0**.

  **Neither Swift nor Rust appears in Appendix G.** Both languages in your system are
  outside the RFC's own verified-compatible set. That reframes §1.1–§1.4: the four Rust
  crates I measured are *all* third-party and none carries the RFC's verification claim.
  The two that matched the reference on every number vector (§1.1.6) did so on their own
  merit, not on any published conformance status.
- The only Swift JCS repo found was `minacle/swift-jcs`: 1 commit, 0 stars, whose README
  states it was *"written entirely with the assistance of AI and has not been tested in a
  real-world environment."* I confirmed **the repository exists and is reachable**
  `[MEASURED: URL resolves]`; I did **not** audit its code. Everything else found
  (`JWSETKit`, `swift-jose`) is sorted-keys, not JCS `[2ND]`.

If this holds, the Swift side of your system is not choosing between JCS implementations —
it is writing one, against a spec whose own reference points (V8, Ryu) are C++ and whose
number rules fight Swift's `Double.description`.

---

# PART 2 — Residual conflict classes

Per your instruction: no CRDT survey. Your merge rule (min/union/append) is already
commutative, associative and idempotent, so convergence *of the merge itself* is not at
issue. What follows is what the merge rule does not cover.

**All of Part 2 is `[2ND]` unless marked otherwise** — sourced by a delegated researcher
from primary documents. I independently verified the four anchors marked `[SPEC]`/`[PAPER]`
below. The rest are leads with citations, not claims I have checked.

## 2.1 The three you named

### (a) Tombstone vs promotion — the named term is **causal stability**

| Finding | Source | Grade |
|---|---|---|
| The exact term for "GC is only safe once all replicas have seen the delete" is **causal stability**, Def. 5.1 | Baquero, Almeida, Shoker, *Pure Operation-Based Replicated Data Types*, **arXiv:1710.04469** — title/identifier verified | `[PAPER]` + `[2ND]` for the quote |
| Cost: the stability oracle needs the **full node set** and a later message from every other node — unsatisfiable under churn | same, §7.2 | `[2ND]` |
| Same constraint restated for content-addressed DAGs: "discarding parts of the Merkle-DAG should not be attempted before making sure that every replica is aware of them… a system constraint that we did not have before" | *Merkle-CRDTs: Merkle-DAGs meet CRDTs*, **arXiv:2004.00107** — title/identifier verified | `[PAPER]` + `[2ND]` for quote |
| Canonical statement of the trade-off: "an 'add to cart' operation is never lost. **However, deleted items can resurface.**" | Dynamo, SOSP 2007 | `[2ND]` |
| Production vocabulary: "resurrected as a **zombie**"; `gc_grace_seconds`; node down > grace ⇒ "deleted data will be **repaired back**" | Apache Cassandra tombstones doc | `[2ND]` |
| Riak: "we recommend setting `delete_mode` to **`keep`** if you plan to delete and recreate objects under the same key… a deleted object may be **resurrected**" — note this is *exactly* delete-then-re-promote-same-digest | Riak Object Deletion Reference | `[2ND]` |
| The GC-vs-concurrent-write race is **explicitly unsolved** in Git: mitigations "**fall short of a complete solution**" | `git-gc(1)` NOTES | `[2ND]` |
| Heavier fix, stated cost: "ensure that the registry is in read-only mode or not running at all… known as **stop-the-world** garbage collection" | CNCF Distribution GC doc | `[2ND]` |

Relevance: your digest identity means a re-promotion after deletion is **bit-identical** to
the original, so there is no version vector to distinguish "stale replica replaying an old
record" from "user legitimately re-added the same content." Riak's `delete_mode=keep`
recommendation is aimed at precisely that case.

### (b) Supersession cycles / never-arriving target

The important structural point, and it is `[2ND]` + `[INFERRED]`, flagged as **not named in
any literature the researcher could find**:

Merkle-DAG acyclicity is not a theorem — it is a hardness argument that depends on **every
edge being inside the parent's hash preimage**. A supersession pointer references a digest
that is *not* a child of the record, so the argument does not cover it, and two records can
mutually supersede. **No literature names this.** Treat as an open question.

Closest documented analogues `[2ND]`:

- **OCI `subject` field** — the best structural analogue. The spec **mandates tolerating
  dangling references**: "A registry MUST initially accept… a `subject` field that
  references a manifest that does not exist… in either order." And disclaims the hard part:
  "**Protection against race conditions is the responsibility of clients and end users.**"
- **Nix** — where references are *not* hash-of-child, acyclicity must be **imposed**:
  "References other than a self-reference must not form a cycle." Plus the closure property.
- **Git promisor objects** — the "expected missing" pattern: packs "may contain trees or
  tags that reference missing blobs", marked `<name>.promisor`, with the candid admission
  that "no check as to whether the missing object is actually a promisor object is performed."
- **OSTree** — `.commitpartial` for legitimate incompleteness; fsck can "add tombstone
  commit for referenced but missing commits."

### (c) Epoch rollback after key compromise — the named treatment is **TUF fast-forward attack**

I verified this one directly `[SPEC]`, because it is the closest match to your exact scenario.

TUF specification §5.3.11, verbatim, which I read:

> If the timestamp and / or snapshot keys have been rotated, then delete the trusted
> timestamp and snapshot metadata files. This is done in order to recover from
> **fast-forward attacks** after the repository has been compromised and recovered. A
> fast-forward attack happens when attackers arbitrarily increase the version numbers of:
> (1) the timestamp metadata, (2) …

That is your "epoch rollback after key compromise and re-enrollment," named and specified:
the documented treatment is to **deliberately discard the counter state the compromised key
controlled**, because the counter itself is no longer trustworthy.

Related TUF text I read `[SPEC]`:

- §5.3 rollback check: "The version number of the new root metadata (version N+1) MUST be
  **exactly** the version in the trusted root metadata (version N) incremented by one" —
  skipping versions is forbidden, which forces clients to walk every intermediate root.
- §5.4.3 timestamp monotonicity, with equal-version ⇒ discard and abort.
- Named attack taxonomy: **Rollback**, **Indefinite freeze**, **Fast-forward**,
  **Mix-and-match**.

<https://theupdateframework.github.io/specification/latest/>

Cost data `[2ND]`: SUNDR's fork consistency is "the strongest notion of integrity possible
without on-line trusted parties"; CONIKS auditing costs ~17.6 kB/day/client; ROTE reports
SGX monotonic counter increments at **~80–250 ms** with **wear-out at ~1.05M writes**
("at one increment per minute, the counters are exhausted in two years"), and a quorum
replacement costing 20–25% throughput. **All ROTE/CONIKS/SUNDR numbers are `[2ND]` and
unverified by me.**

## 2.2 Residual classes you did **not** name

You said this would be the most valuable output. These are `[2ND]` with primary-source
citations; I verified the two marked.

| # | Class | How it bites this design | Source | Grade |
|---|---|---|---|---|
| **R1** | **NFC-before-JCS is spec-contrary** | RFC 8785 §3.1 requires preserving strings "as is"; normalize-then-canonicalize ≠ canonicalize-then-normalize, and nothing enforces step order across two codebases | RFC 8785 §3.1 | `[SPEC]` **verified** |
| **R2** | **Ed25519 validity divergence between Swift and Rust libraries** | Bundles signed on one device may be **rejected** on the other, or vice versa. Cofactored vs cofactorless verification, non-canonical S, small-order points. In a *one-directional* replica there is no back-channel, so this is silent permanent starvation | *Taming the many EdDSAs*, eprint 2020/1244 — **abstract verified**: "showed that most libraries do not comply with the latest standardization recommendations… of practical importance for consensus-driven applications". Test vectors: github.com/novifinancial/ed25519-speccheck — **URL verified** | `[PAPER]` **verified**; specific CryptoKit-vs-dalek vector numbers are `[2ND]` |
| **R3** | **Unicode/ICU version skew changes NFC output** | Foundation uses the OS's ICU (moves with macOS updates); Rust's `unicode-normalization` pins its Unicode version at compile time. Same input, two NFC results, two digests. UAX #15 stability holds only for codepoints **assigned in both versions** | UAX #15 Normalization Stability Policy | `[2ND]`, and the researcher labelled the macOS-vs-Rust instance an **inference** — no filed bug found |
| **R4** | **`union(tags)` is a G-Set: untagging is impossible** | Tag removal cannot be represented at all. Named limitation; has a compliance dimension (erasure requests) | Shapiro et al. CRDT taxonomy §3.3.1 | `[2ND]` |
| **R5** | **`min(createdAt)` is irreversible** | One device with a bad clock permanently poisons `createdAt` with **no repair path** — unlike LWW, where a later correct write recovers. Min has no inverse | aphyr/Jepsen clock-skew writeups; Cloudflare leap-second postmortem | `[2ND]` — the researcher explicitly **failed to find** a Cassandra clock-skew Jepsen report and discarded that citation |
| **R6** | **`append(sightings)` grows without bound; adding compaction converts it into a correctness bug** | Unbounded metadata growth; but a retention horizon means "append" is no longer idempotent-safe for late-arriving duplicates | Kafka log compaction; event-sourcing compaction | `[2ND]` |
| **R7** | **Enrichment/embeddings excluded from digest ⇒ silent winner on merge** | Two records, same digest, different enrichment: the merge rule is **silent** on which enrichment survives. This is a genuine gap in the stated rule, not an implementation issue | RFC 9110 weak-validator analogy | `[2ND]` — analogy is the researcher's framing, not a literature term |
| **R8** | **No hash-algorithm agility** | Supersession references are digest-typed, so a hash migration rewrites every reference. Git's SHA-1→SHA-256 transition needed a bidirectional translation table *and* a new `gpgsig-sha256` field | Git hash-function-transition doc | `[2ND]` |
| **R9** | **Idempotence is scoped to an epoch** | Kafka KIP-98 states idempotence holds "only within a single producer session." Your promotion key is `(epoch, seq)` — so the **epoch boundary**, not just rollback, is where at-least-once becomes at-least-twice | KIP-98 | `[2ND]` |
| **R10** | **Dual-write across two heterogeneous stores** | Core Data commit + SQLite commit are not one transaction; partial bundle application on either side. Documented as the dual-write problem, with the outbox pattern as the standard treatment | dual-write / transactional outbox literature | `[2ND]` |
| **R11** | **SQLite collation splits `contentDomain`** | `BINARY` vs `NOCASE` vs ICU collations give different uniqueness semantics for the domain half of your identity pair; embedded NUL truncates SQLite comparison | SQLite datatype/collation docs | `[2ND]` |
| **R12** | **UAX15-D4 stream-safe CGJ insertion** | Stream-safe normalization can *insert* a combining grapheme joiner into long combining sequences, changing bytes. Applies only to pathological inputs | UAX #15 | `[2ND]` |

**Explicitly discarded as unsourced** (the researcher looked and found nothing — recording
so you don't re-spend the effort): SHA-256 collision as a practical concern;
"read-only replica drift" as a literature term; a filed macOS-vs-Rust NFC divergence bug;
a Cassandra clock-skew Jepsen report; "DVCS resurrection" as a term of art;
Signal/Matrix spec text for epoch rollback after device re-enrollment.

---

## 3. Self-consistency check

I re-read this artifact for internal contradictions, as instructed. Findings:

1. **§1.4 vs delegated report on duplicate keys** — a real contradiction. Resolved in
   §1.4 in favour of my own measurement, with the scope difference (`Value` path vs
   `serialize_map` path) stated explicitly and the unverified half left flagged.
2. **§1.2.1 vs delegated report on what issue #1 demonstrates** — a real contradiction.
   Resolved in favour of the issue thread I read; both defects are separately measured.
3. **§0.1 (JCS forbids NFC) vs §R1 (NFC-before-JCS is spec-contrary)** — checked; these
   agree. §0.1 says JCS must not normalize internally; R1 says the *pipeline* ordering is
   unenforced across two codebases. Not the same claim.
4. **§1.1.5 (json-canon emits `null` for NaN) vs §1.1.2 (json-canon errors on >2^53)** —
   checked; not contradictory. Different code paths: integer range check vs float
   formatter. Both measured on the same binary.
5. **§1.3 (no lone-surrogate divergence) vs §1.5 (major Unicode divergence)** — checked;
   consistent. §1.3 is about *ill-formed* UTF-16, which both languages' string types
   exclude by construction. §1.5 is about *well-formed but non-identical* sequences, which
   both languages accept and treat differently.

No unresolved contradictions remain. Two claims are knowingly left in tension **with
themselves flagged**: the Swift `Double.description` thresholds (§1.1.6) and the Swift
lone-surrogate decoder behaviour (§1.3) are stated as open questions, not findings.

---

## 4. What I could not do

- **No Swift toolchain on this machine.** Every Swift behavioural claim is source-read or
  second-hand. The three highest-value unmeasured items: `(-0.0).description`,
  `Double.description` exponential thresholds, and `JSONDecoder` lone-surrogate handling.
- **Did not verify** the cyberphone conformance-corpus pass claim, crates.io download
  figures, `canon-json` and `vr-jcs` behaviour, or the `serialize_map` duplicate-key split.
- **Did not audit** `minacle/swift-jcs` source.
- Part 2 is documented-only throughout; nothing in it was executed.

---

## Sources

**Specifications (read directly)**
- RFC 8785, JSON Canonicalization Scheme — <https://www.rfc-editor.org/rfc/rfc8785.txt> (§3.1, §3.2.2.2, §3.2.2.3, §3.2.3, App. B, App. G)
- The Update Framework specification — <https://theupdateframework.github.io/specification/latest/> (§5.3, §5.3.11, §5.4.3)
- Swift `String` documentation — <https://developer.apple.com/documentation/swift/string>

**Source code (read directly)**
- swift-foundation `JSONWriter.swift` — <https://github.com/swiftlang/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONWriter.swift>
- swift-foundation `JSONEncoder.swift` — <https://github.com/swiftlang/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONEncoder.swift>
- Swift stdlib `String.swift` — <https://github.com/apple/swift/blob/main/stdlib/public/core/String.swift>

**Issues (read directly)**
- serde_jcs #1, "Sorting properties as unescaped UTF-16" (open) — <https://github.com/l1h3r/serde_jcs/issues/1>
- Swift SR-3754 / apple/swift#46339, K vs Kelvin sign — <https://github.com/apple/swift/issues/46339>

**Papers (identifier + title/abstract verified)**
- Chalkias, Garillot, Nikolaenko, *Taming the many EdDSAs*, SSR 2020 — <https://eprint.iacr.org/2020/1244>
- Sanjuán, Pöyhtäri, Teixeira, Psaras, *Merkle-CRDTs: Merkle-DAGs meet CRDTs*, 2020 — arXiv:2004.00107
- Baquero, Almeida, Shoker, *Pure Operation-Based Replicated Data Types*, 2017 — arXiv:1710.04469
- Ed25519 edge-case test vectors — <https://github.com/novifinancial/ed25519-speccheck>

**Measured artifacts (this machine)**
- `experiments/jcs-divergence/src/main.rs` — probe source
- `experiments/jcs-divergence/results.txt` — raw output, rustc 1.97.1

**Delegated research notes (unverified detail, `[2ND]` throughout)**
- `notes/rust-jcs-findings.md`, `notes/swift-jcs-findings.md`, `notes/conflict-precedent.md`, `notes/residual-gaps.md`
