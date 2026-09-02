# Swift implementations of RFC 8785 (JCS) and Swift-side canonical-JSON options

Research date: 2026-08-06. All source trees below were cloned/fetched and read directly unless marked INFERRED.

---

## Executive summary (one paragraph)

There is **no ecosystem-grade Swift JCS implementation**. RFC 8785 Appendix G lists five languages and Swift is not among them [1]; `cyberphone/json-canonicalization`'s own implementation table lists nine languages and Swift is not among them [2]. The only Swift repository on GitHub that self-describes as an RFC 8785 implementation is `minacle/swift-jcs`, a single-commit, 0-star, self-declared **AI-written and untested** package not listed in the Swift Package Index [3][4]. Everything else in Swift is `JSONEncoder.outputFormatting = .sortedKeys`, which is **not** JCS. Three concrete divergence classes make a naive Swift port of a Rust JCS implementation wrong: **(b)** Swift's `Double.description` is *not* ECMAScript `Number::toString` — it switches to exponential form at 2^53 instead of 1e21, at 10^-5 instead of 10^-7, zero-pads exponents to two digits, and always emits `.0`; **(c)** `JSONEncoder.sortedKeys` sorts by **UTF-8 code units** (Linux/swift-foundation) or by an **NSString locale-aware `.numeric/.caseInsensitive/.forcedOrdering` compare** (legacy Apple/corelibs path), neither of which is the UTF-16 code-unit order RFC 8785 mandates; **(d)** Swift's `String ==`, `<`, and `hash(into:)` are all **NFC-normalizing**, so `[String: Any]` silently merges canonically-equivalent JSON member names that Rust's `BTreeMap<String, _>` / `serde_json::Map` keeps distinct. (d) is the largest Swift/Rust semantic gap.

---

## Evidence table

| # | Source | URL | Key claim | Type | Confidence |
|---|--------|-----|-----------|------|------------|
| 1 | RFC 8785 Appendix G, "Open-Source Implementations" (read from rfc8785.txt) | https://www.rfc-editor.org/rfc/rfc8785.txt | Lists exactly JavaScript, Java, Go, .NET/C#, Python. No Swift. `grep -i swift rfc8785.txt` → 0 hits. | primary | high |
| 2 | cyberphone/json-canonicalization README (repo cloned) | https://github.com/cyberphone/json-canonicalization | Implementations table: Rust, JavaScript, Java, Go, .NET/C#, Python, Elixir, Ruby, PHP. No Swift. No `swift/` directory in tree. | primary | high |
| 3 | minacle/swift-jcs (repo cloned, full read) | https://github.com/minacle/swift-jcs | Only Swift repo self-describing as RFC 8785. 1 commit `1e69bef` (2026-04-04), tag v0.1.0, 0 stars, Unlicense. README: "The code in this project was written entirely with the assistance of AI and has not been tested in a real-world environment." | primary | high |
| 4 | SwiftPackageIndex/PackageList packages.json | https://raw.githubusercontent.com/SwiftPackageIndex/PackageList/main/packages.json | No JCS / canonical-JSON package indexed (`grep -i "jcs\|canonical"` returns only JCState, CanonicalPackageURL, swift-canonical-filepath, MachOObjCSection). | primary | high |
| 5 | swiftlang/swift `stdlib/public/core/FloatingPointToString.swift` | https://github.com/swiftlang/swift/blob/main/stdlib/public/core/FloatingPointToString.swift | Design doc comment + `_finishFormatting` + `forceExponential` logic. | primary | high |
| 6 | swiftlang/swift `test/stdlib/PrintFloat.swift.gyb` (official test suite) | https://github.com/swiftlang/swift/blob/main/test/stdlib/PrintFloat.swift.gyb | Golden expected `description` strings for Double, incl. exponential thresholds and 2-digit exponents. | primary | high |
| 7 | ECMA-262 6th ed. §7.1.12.1 "ToString Applied to the Number Type" | https://262.ecma-international.org/6.0/#sec-tostring-applied-to-the-number-type | The exact algorithm RFC 8785 §3.2.2.3 normatively references. | primary | high |
| 8 | RFC 8785 Appendix B, Table 1 (number serialization samples) | https://www.rfc-editor.org/rfc/rfc8785.txt | 24 IEEE-754 → JSON golden pairs including `-0` → `0`, `9007199254740992`, `295147905179352830000`, `0.000001`, `9.999999999999997e-7`. | primary | high |
| 9 | apple/swift-foundation `Sources/FoundationEssentials/JSON/JSONWriter.swift` (repo cloned @ f0442fb) | https://github.com/apple/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONWriter.swift | `.sortedKeys` sorts via `a.key.utf8.lexicographicallyPrecedes(b.key.utf8)`; a `compatibility1` branch uses NSString locale compare. | primary | high |
| 10 | apple/swift-foundation `Sources/FoundationEssentials/JSON/JSONEncoder.swift` | https://github.com/apple/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONEncoder.swift | Float encoding = `float.description` with trailing `".0"` stripped; object payload type is `[String: JSONEncoderValue]`. | primary | high |
| 11 | swiftlang/swift-corelibs-foundation `Sources/Foundation/JSONSerialization.swift` (cloned) | https://github.com/swiftlang/swift-corelibs-foundation/blob/main/Sources/Foundation/JSONSerialization.swift | `JSONSerialization.WritingOptions.sortedKeys` uses `a.compare(b, options: [.numeric, .caseInsensitive, .forcedOrdering], range:, locale: NSLocale.system)`. | primary | high |
| 12 | swiftlang/swift `stdlib/public/core/StringComparison.swift` | https://github.com/swiftlang/swift/blob/main/stdlib/public/core/StringComparison.swift | `_slowCompare` iterates `unicodeScalars._internalNFC` and compares scalar values; fast path is `memcmp` of UTF-8 when both are NFC. | primary | high |
| 13 | swiftlang/swift `stdlib/public/core/StringHashable.swift` | https://github.com/swiftlang/swift/blob/main/stdlib/public/core/StringHashable.swift | `String.hash(into:)` hashes NFC-normalized code units (`_withNFCCodeUnits` / `_normalizedHash`). | primary | high |
| 14 | swiftlang/swift `stdlib/public/core/String.swift` doc comment | https://github.com/swiftlang/swift/blob/main/stdlib/public/core/String.swift | "Comparing strings for equality using the equal-to operator (`==`) or a relational operator (like `<` or `>=`) is always performed using Unicode canonical representation." | primary | high |
| 15 | RFC 8785 §3.2.3 "Sorting of Object Properties" | https://www.rfc-editor.org/rfc/rfc8785.txt | "Property name strings to be sorted are formatted as arrays of UTF-16 [UNICODE] code units." + note that UTF-8/UTF-32 sorting "would differ and thus be incompatible with this specification." | primary | high |
| 16 | Swift issue #46339 (SR-3754) | https://github.com/apple/swift/issues/46339 | `"K"` (U+004B) `==` `"K"` (U+212A KELVIN SIGN) is `true` in Swift; not true for NSString. | primary | high |
| 17 | Swift Forums, "Double to String conversion implementation" (Nov 2025), reply by xwu | https://forums.swift.org/t/double-to-string-conversion-implementation/83335 | "since Swift 4.2, the standard library has used a variation of the Grisu2 algorithm with changes described in Errol3… minimum number of digits required for lossless conversion"; quotes PR #15474 showing `1e+23`, `9.999999999999997e+22`, `1.0000000000000001e+23`. | primary (forum, by Swift core contributor) | high |
| 18 | swiftlang/swift PR #15474 (SR-106) | https://github.com/swiftlang/swift/pull/15474 | Original `description` reimplementation; "Always Accurate / Always Short / Always Close". | primary | high |
| 19 | apple/swift PR #35299 — SwiftDtoa v2 | https://github.com/apple/swift/pull/35299 | "SwiftDtoa is the C/C++ code used in the Swift runtime to produce the textual representations used by the `description` and `debugDescription` properties… does not change the actual output." | primary | high |
| 20 | amosavian/JWSETKit `Sources/JWSETKit/Cryptography/Keys.swift` | https://github.com/amosavian/JWSETKit/blob/main/Sources/JWSETKit/Cryptography/Keys.swift | RFC 7638 thumbprint uses `JSONEncoder` with `outputFormatting = [.sortedKeys, .withoutEscapingSlashes]` — sorted keys, not JCS. | primary | high |
| 21 | mattt/swift-yyjson PR #14 | https://github.com/mattt/swift-yyjson/pull/14 | Adds `.sortedKeys` to YYJSONEncoder, "Keys are sorted in UTF-8 byte order, same as Foundation's `JSONEncoder`", implemented with `strcmp`. | primary | high |
| 22 | apple/swift-foundation issue #284 | https://github.com/apple/swift-foundation/issues/284 | "Make String.compare with locale argument available in FoundationInternationalization" — the issue referenced by the historical `// TODO: Reenable once String.compare is implemented` guard around `sortedKeys`. | primary | medium |
| 23 | proxyco/swift-jose | https://github.com/proxyco/swift-jose | JOSE package (JWS/JWE/JWK/JWA/RFC7638/RFC8037/COSE). No RFC 8785 in its stated spec list. | primary | medium |
| 24 | Swift Forums — ligature/canonical-vs-compatibility thread | https://forums.swift.org/t/swift-string-comparison-doesnt-consider-ligatures-equivalent-to-their-components/66665 | Confirms Swift promises *canonical* (not compatibility) equivalence: "ﬃ" ≠ "ffi", but "caña" (NFC) == "caña" (NFD). | secondary (forum) | medium |
| 25 | Swift.org blog, "UTF-8 String" | https://www.swift.org/blog/utf8-string/ | Swift 5 String native storage is UTF-8; UTF-16 view is provided via amortized "breadcrumbs". Validation-on-creation, not normalization-on-creation. | primary | high |

---

## Findings

### (a) Swift JCS implementations found

#### A1. `minacle/swift-jcs` — the only real Swift JCS implementation on GitHub

| Field | Value |
|---|---|
| Name | `swift-jcs`, library product `JSONCanonicalization` |
| Repo | https://github.com/minacle/swift-jcs |
| Version | tag `v0.1.0` |
| Last commit | `1e69befe76f5445696e821811402c586dd2186d8`, "Initialise JSONCanonicalization Swift package", 2026-04-04 (single commit in history) |
| Stars / maintenance | 0 stars, created 2026-04-03, pushed 2026-04-03, not archived, Unlicense |
| In Swift Package Index? | **No** — absent from `PackageList/packages.json` [4] |
| Real JCS or "sorted keys"? | **Real JCS attempt** — implements ECMA-262 §7.1.12.1 number formatting *and* UTF-16 code-unit key sorting |
| Size | 289 source LOC + 414 test LOC |
| Swift requirement | `swift-tools-version: 6.3`, `swiftLanguageModes: [.v6]` (very new; excludes most toolchains) |

Its own README carries this warning verbatim [3]:

> `> [!WARNING]`
> `The code in this project was written entirely with the assistance of AI and has not been tested in a real-world environment.`

**What it gets right.** Key sorting is a genuine UTF-16 code-unit comparison, not `String <` [3]:

```swift
internal func _compareUTF16(_ a: String, _ b: String) -> Bool {
    let aUTF16 = Array(a.utf16)
    let bUTF16 = Array(b.utf16)
    let minLength = min(aUTF16.count, bUTF16.count)
    for i in 0..<minLength {
        if aUTF16[i] != bUTF16[i] {
            return aUTF16[i] < bUTF16[i]
        }
    }
    return aUTF16.count < bUTF16.count
}
```

Number serialization takes the correct architectural approach: harvest Swift's shortest-round-trip *digits* via `String(Double)` and then **re-format them per ECMAScript rules**, rather than trusting Swift's formatting [3]:

```swift
    let (digits, n) = _extractDigitsAndExponent(absValue)
    let k = digits.count
    // ECMAScript formatting rules (ECMA-262 §7.1.12.1 steps 5-9):
    let result: String
    if k <= n && n <= 21 { ... }
    else if 0 < n && n < k { ... }
    else if -6 < n && n <= 0 { ... }
    else { /* exponential, "e" + sign + Int(exponent) with no zero-padding */ }
```

`_serializeNumber` also special-cases zero to `"0"`, correctly handling `-0.0` per RFC 8785 Appendix B [3][8].

**Concrete defects I identified by reading the source (severity noted):**

- **HIGH — canonical-equivalence key collapse.** `_serialize` matches `case let dict as [String: Any]` and does `dict.keys.sorted(by: _compareUTF16)` then `dict[key]!`. Because it goes through a Swift `Dictionary<String, Any>`, two JSON member names that are distinct UTF-16 sequences but canonically equivalent (e.g. `"Café"` vs `"Cafe\u0301"`) are **already merged before sorting**. This is a silent-data-loss / signature-mismatch class bug relative to a Rust `serde_json::Map` implementation. See finding (d).
- **HIGH — all numbers routed through `Double`.** `_serializeNumber(number.doubleValue)` means an `Int64` outside ±2^53 loses precision. RFC 8785 §3.2.2.3 is Double-based [8], but I-JSON / `NSNumber(Int64)` inputs will silently round. This differs from Rust JCS crates that preserve integer tokens.
- **MEDIUM — incorrect provenance comment.** The code comments claim "Uses Swift's `String(Double)` which produces the shortest round-trippable decimal representation (**Ryu** algorithm)". Swift does **not** use Ryu; it uses a modified Grisu2 with Errol3 refinements (SwiftDtoa) [17][18][19]. Cosmetic, but signals unverified authorship.
- **UNVERIFIED — no conformance run.** I did not execute the test suite (no Swift toolchain in this environment). Its 414-line test file exists; whether it covers RFC 8785 `testdata/` vectors is **not verified**.
- **File paths for review:** `Sources/JSONCanonicalization/JSONCanonicalization.swift` (sorting + dispatch), `Sources/JSONCanonicalization/NumberSerializer.swift` (ECMA-262 number rules), `Sources/JSONCanonicalization/StringSerializer.swift` (escaping).

#### A2. Everything else Swift-side is "sorted keys", not JCS

| Name | URL | Status | JCS or sorted-keys? |
|---|---|---|---|
| `JSONEncoder.outputFormatting.sortedKeys` (apple/swift-foundation) | https://github.com/apple/swift-foundation | Actively maintained (main @ `f0442fb`, 2026-08-05) | **Sorted keys only.** UTF-8 order [9]. Not JCS. |
| `JSONSerialization.WritingOptions.sortedKeys` (swift-corelibs-foundation) | https://github.com/swiftlang/swift-corelibs-foundation | Maintained (main, 2026-07-22) | **Sorted keys only,** and via a *locale-aware, case-insensitive, numeric* NSString compare [11]. Far from JCS. |
| `amosavian/JWSETKit` RFC 7638 thumbprint | https://github.com/amosavian/JWSETKit | Active | Sorted keys via `JSONEncoder` [20]. RFC 7638 only requires lexicographic ordering of an ASCII-only key set, so this is spec-valid for 7638 but is **not** a JCS canonicalizer. |
| `mattt/swift-yyjson` `.sortedKeys` (PR #14, merged 2026-02-05) | https://github.com/mattt/swift-yyjson/pull/14 | Active | Sorted keys, "UTF-8 byte order, same as Foundation's JSONEncoder", via `strcmp` [21]. Not JCS; `strcmp` additionally truncates at embedded NUL (noted in the PR's own review comment). |
| `proxyco/swift-jose` | https://github.com/proxyco/swift-jose | Its README lists RFC7515/7516/7517/7518/7638/8037/COSE — **no RFC 8785** [23] | Not a JCS implementation. |
| `CharlZKP/json-stringify-deterministic-swift` | https://github.com/CharlZKP/json-stringify-deterministic-swift | 0 stars, 2026-02-06 | Port of the JS `json-stringify-deterministic` lib, a *different* determinism scheme, **not** RFC 8785. |

GitHub repository search (`language:swift` × {`jcs`, `json canonicalization`, `canonicalize json`, `canonical json`, `rfc8785`, `jws jcs`}) returned `minacle/swift-jcs` as the sole match with a JCS description; `rfc8785+language:swift` returned **0** repositories.

Other Swift code that mentions RFC 8785 exists only as vendored fragments inside larger polyglot projects (e.g. `ai-university-aiu/causalontology` `bindings/swift/Sources/Causalontology/Jcs.swift`, which states in its own header that "full ECMAScript exponent formatting for extreme magnitudes is pinned at the 1.0.0 conformance freeze"). These are not distributable packages and I did not read them in depth — **INFERRED from search snippets only, treat as unverified.**

---

### (b) Number formatting: exactly where Swift `Double.description` diverges from ECMAScript `Number::toString`

**Both algorithms produce the same *digits*.** Swift's `description` is "Always Short" and "Always Close" (shortest round-trip, ties-to-even on the last digit) [5][18], which is exactly ECMA-262 §7.1.12.1 step 5 as refined by NOTE 2 [7] — the same "Note 2" enhancement RFC 8785 §3.2.2.3 makes mandatory [8]. **The divergence is entirely in the *presentation layer*, i.e. `_finishFormatting`.**

The Swift stdlib design comment states the presentation goals explicitly [5]:

```
/// Beyond the requirements above, the precise text form has been
/// tuned to try to maximize readability:
/// * Always include a '.' or an 'e' so the result is obviously
///   a floating-point value
/// * Exponential form always has 1 digit before the decimal point
/// * When present, a '.' is never the first or last character
/// * There is a consecutive range of integer values that can be
///   represented in any particular type (-2^54...2^54 for double).
///   We do not use exponential form for integral numbers in this
///   range.
/// * Generally follow existing practice: Don't use use exponential
///   form for fractional values bigger than 10^-4; always write at
///   least 2 digits for an exponent.
```

Compare ECMA-262 §7.1.12.1 [7]:

> 6. If k ≤ n ≤ 21, return the String consisting of the code units of the k digits of the decimal representation of s (in order, with no leading zeroes), followed by n−k occurrences of the code unit 0x0030 (DIGIT ZERO).
> 7. If 0 < n ≤ 21, return … most significant n digits … 0x002E (FULL STOP) … remaining k−n digits …
> 8. If −6 < n ≤ 0, return … 0x0030 … 0x002E … −n occurrences of 0x0030 … the k digits …
> 9. Otherwise, if k = 1, return … 0x0065 (LATIN SMALL LETTER E) … 0x002B / 0x002D … **the decimal representation of the integer abs(n−1) (with no leading zeroes)**.
> 2. If m is +0 or −0, return the String "0".

#### Divergence class D1 — trailing `.0` on integral values

Swift **always** emits a `.` or an `e` [5]. Official test suite [6]:

```swift
  expectDescription("1.0", asFloat64(1.0))
  expectDescription("125.0", asFloat64(125.0))
  expectDescription("1250000000000000.0", asFloat64(1250000000000000.0))
```
ECMAScript: `1`, `125`, `1250000000000000` (step 6, `k ≤ n ≤ 21`) [7].

**Mitigated in Foundation but not in the stdlib.** `swift-foundation`'s JSONEncoder strips it [10]:
```swift
        var string = float.description
        if string.hasSuffix(".0") {
            string.removeLast(2)
        }
        return .number(string)
```
A hand-rolled canonicalizer that calls `String(d)` directly and forgets this strip will emit `4.5` correctly but `1.0` wrongly.

#### Divergence class D2 — large-magnitude threshold: **2^53, not 1e21** (the biggest number bug)

Swift's `_Float64ToASCII` computes [5]:
```swift
  let isBoundary = (f.significandBitPattern == 0)
  let forceExponential =
    ((binaryExponent > 1)
       || (binaryExponent == 1 && !isBoundary))
```
with `binaryExponent = rawExponent − 1075`, i.e. the value equals `significand(53-bit int) × 2^binaryExponent`. This forces exponential form for every finite value strictly greater than 2^53 in magnitude. The official test suite pins the exact boundary [6]:

```swift
  // Double can represent all integers -(2^53)...(2^53)
  let maxDecimalForm = Double((1 as Int64) << 53)
  expectDescription("9007199254740992.0", maxDecimalForm)
  expectDescription("-9007199254740992.0", -maxDecimalForm)
  // Outside of that range, we use exponential form:
  expectDescription("9.007199254740994e+15", maxDecimalForm.nextUp)
  expectDescription("-9.007199254740994e+15", -maxDecimalForm.nextUp)
```
and, for powers of ten [6]:
```swift
  for power in lowerBound ... 308 {
    if power < -4 || power > 15 { // Exponential form
```
plus [6]:
```swift
  expectDescription("1.25e+17", asFloat64(125000000000000000.0))
  expectDescription("1.25e+16", asFloat64(12500000000000000.0))
  expectDescription("1250000000000000.0", asFloat64(1250000000000000.0))
```

ECMAScript switches to exponential only when `n > 21`, i.e. at 1e21 [7]. So **every finite double in the magnitude window (2^53, 1e21) is formatted differently.** RFC 8785 Appendix B contains three test vectors that land squarely inside this window [8]:

| IEEE 754 hex | RFC 8785 required output [8] | Swift `Double.description` (derived from [5][6]) | Match? |
|---|---|---|---|
| `4340000000000000` (2^53) | `9007199254740992` | `9007199254740992.0` | ✗ (D1 only; JSONEncoder's strip fixes it) |
| `4430000000000000` (~2^68) | `295147905179352830000` | `2.9514790517935283e+20` | ✗ **structural** |
| `444b1ae4d6e2ef4e` | `999999999999999700000` | `9.999999999999997e+20` | ✗ **structural** |
| `444b1ae4d6e2ef4f` | `999999999999999900000` | `9.999999999999999e+20` | ✗ **structural** |
| `444b1ae4d6e2ef50` | `1e+21` | `1e+21` | ✓ |

**Answer to the specific question in the task: `1e21` is a MATCH.** Swift prints `1e+21` and ECMAScript prints `1e+21`. The 1e21 boundary is the one place they coincide; the entire decade *below* it is where they diverge. (Derivation for `295147905179352830000`: RFC's own required output is the 21-digit fixed form because n = 21 ≤ 21; Swift's `binaryExponent` for 2^68 is 16 > 1 ⇒ `forceExponential`. Marked derived-from-code + test-suite pattern; I did not execute Swift.)

#### Divergence class D3 — small-magnitude threshold: **10^-5, not 10^-7**

`_finishFormatting` [5]:
```swift
  var p = base10Power &+ digitCount &- 1
  if p < -4 || forceExponential {
```
Test suite [6]:
```swift
  expectDescription("0.000125", asFloat64(0.000125))
  expectDescription("1.25e-05", asFloat64(0.0000125))
  expectDescription("1.25e-06", asFloat64(0.00000125))
  expectDescription("1.25e-07", asFloat64(0.000000125))
```
ECMAScript uses fixed form while `−6 < n ≤ 0`, i.e. down to and including 1e-6 [7]. RFC 8785 Appendix B [8]:

| IEEE 754 hex | RFC 8785 required [8] | Swift (derived) | Match? |
|---|---|---|---|
| `3eb0c6f7a0b5ed8d` | `0.000001` | `1e-06` | ✗ **structural** |
| `becbf647612f3696` | `-0.0000033333333333333333` | `-3.3333333333333333e-06` | ✗ **structural** |
| `3eb0c6f7a0b5ed8c` | `9.999999999999997e-7` | `9.999999999999997e-07` | ✗ (D4 padding) |

#### Divergence class D4 — exponent zero-padding to 2 digits

Swift, in `_finishFormatting` [5]:
```swift
    // For historical reasons, exponents are always at least 2 digits
    let d = unsafe asciiDigitTable[unchecked: p]
    buffer.storeBytes(of: d, toByteOffset: nextDigit, as: UInt16.self)
```
and the test-suite helper `exponentialPowerOfTen` builds at least two digits [6]:
```swift
  if (p > 99) { s += digits[ (p / 100) % 10] }
  s += digits[ (p / 10) % 10]
  s += digits[ p % 10]
```
ECMAScript step 9/10: "the decimal representation of the integer abs(n−1) **(with no leading zeroes)**" [7].

⇒ Swift `1e-07`, `1.25e-05`, `9.999999999999997e-07` vs ECMAScript `1e-7`, `1.25e-5`, `9.999999999999997e-7`. This affects **every** value whose decimal exponent magnitude is 1–9, in both directions. Combined with D2/D3 the "safe" exponent range where Swift matches ECMAScript is `|exponent| ≥ 10` on the negative side and exactly the `e+21`…`e+308` band on the positive side.

#### Divergence class D5 — negative zero

Swift test suite [6]: `expectDescription("-0.0", asFloat64(-0.0))`.
ECMA-262 step 2: "If m is +0 or −0, return the String `"0"`" [7]; RFC 8785 Appendix B row `8000000000000000` → `0` [8].
Foundation's `.0`-strip turns this into `-0`, which is **still wrong** for JCS.

#### Divergence class D6 — non-finite values

Swift: `"inf"`, `"-inf"`, `"nan"`, `"nan(0xffff)"`, `"snan"` [6]. RFC 8785 §3.2.2.3: "occurrences of NaN or Infinity MUST cause a compliant JCS implementation to terminate with an appropriate error" [8]. Any canonicalizer built on raw `description` must reject rather than serialize.

#### Cases where Swift and ECMAScript AGREE (verified against RFC Appendix B [8] and the Swift test suite [6][17])

| Value | Both emit |
|---|---|
| `Double.leastNonzeroMagnitude` | `5e-324` — Swift test [6] `expectDescription("5e-324", Double.leastNonzeroMagnitude)`; RFC row `0000000000000001` → `5e-324` [8]. **The `5e-324` case in the task prompt is a MATCH.** |
| `Double.greatestFiniteMagnitude` | `1.7976931348623157e+308` [6][8] |
| `1e23` and neighbours | `9.999999999999997e+22`, `1e+23`, `1.0000000000000001e+23` [17][8] |
| `1e+21`, `1e+30`, `1e-27` | identical [8] |
| `333333333.3333333`, `4.5`, `0.002`, `1424953923781206.2` | identical [2][8] |

#### Summary rule for implementers

> Swift's `Double.description` digits are ECMAScript-correct; Swift's *formatting* is not. The only safe pattern is `minacle/swift-jcs`'s: parse `String(d)` back into `(digits, n)` and re-emit per ECMA-262 §7.1.12.1 steps 6–10, plus special-case `±0.0 → "0"` and reject non-finite. Do **not** use `String(d)` output directly, and do **not** rely on Foundation's `.0`-strip as sufficient.

---

### (c) `JSONEncoder.sortedKeys` — ordering basis

**Yes, the option exists**, and in current `apple/swift-foundation` it is unconditional [10]:
```swift
        /// The output formatting option that sorts keys in lexicographic order.
        @available(macOS 10.13, iOS 11.0, watchOS 4.0, tvOS 11.0, *)
        public static let sortedKeys    = OutputFormatting(rawValue: 1 << 1)
```
(Historically it was gated `#if FOUNDATION_FRAMEWORK` with `// TODO: Reenable once String.compare is implemented — https://github.com/apple/swift-foundation/issues/284` [22]; that guard is gone on `main` @ `f0442fb`, 2026-08-05.)

**Ordering basis — three different answers depending on which Foundation you get.**

**(c-1) Modern swift-foundation, non-Apple-framework path — UTF-8 code-unit order** [9]:
```swift
            // If we didn't use the NSString-based compatibility sorting, sort lexicographically by the UTF-8 view
            if !compatibilitySorted {
                let elems = dict.sorted { a, b in
                    a.key.utf8.lexicographicallyPrecedes(b.key.utf8)
                }
```
`String.utf8` is the string's stored UTF-8 view, so this is a raw byte-order sort.

**(c-2) Apple-framework legacy path — locale-aware NSString compare** [9]:
```swift
            if JSONEncoder.compatibility1 {
                // If applicable, use the old NSString-based sorting with appropriate options
                compatibilitySorted = true
                ...
                    let options: String.CompareOptions = [.numeric, .caseInsensitive, .forcedOrdering]
                    let range = NSMakeRange(0, a.key.length)
                    let locale = Locale.system
                    return a.key.compare(b.key as String, options: options, range: range, locale: locale) == .orderedAscending
```
This is **wildly** non-JCS: `.numeric` makes `"2" < "10"` (JCS requires `"1" < "10" < "2"`), `.caseInsensitive` makes `"A"` and `"a"` adjacent-and-ambiguous, and `Locale.system` introduces collation. `compatibility1` is not defined in the open-source tree — it comes from Apple's closed FOUNDATION_FRAMEWORK build (a linked-on-or-after check). **INFERRED**: its gating condition is a binary-compatibility version check; I could not read its definition.

**(c-3) swift-corelibs-foundation `JSONSerialization` — same legacy NSString compare, unconditionally** [11]:
```swift
        if sortedKeys {
            let elems = try dict.sorted(by: { a, b in
                ...
                let options: NSString.CompareOptions = [.numeric, .caseInsensitive, .forcedOrdering]
                let range: Range<String.Index>  = a.startIndex..<a.endIndex
                let locale = NSLocale.system
                return a.compare(b, options: options, range: range, locale: locale) == .orderedAscending
            })
```
So `JSONSerialization(options: .sortedKeys)` on Linux is **not even byte-deterministic across locales** in principle, let alone JCS-compatible.

**How UTF-8 order differs from the UTF-16 order JCS requires.** RFC 8785 §3.2.3 is explicit [15]:

> Property name strings to be sorted are formatted as arrays of UTF-16 [UNICODE] code units. The sorting is based on pure value comparisons, where code units are treated as unsigned integers, independent of locale settings.

and, in a Note [15]:

> For the purpose of obtaining a deterministic property order, sorting of data encoded in UTF-8 or UTF-32 would also work, but the outcome for JSON data like above would differ and thus be incompatible with this specification.

UTF-8 byte order is order-isomorphic to Unicode **scalar** order. UTF-16 order is scalar order **except** that all non-BMP scalars (U+10000–U+10FFFF) encode as surrogate pairs beginning with 0xD800–0xDBFF, which sort *below* U+E000–U+FFFF. The RFC's own conformance vector demonstrates it [15]:

```
     {
       "\u20ac": "Euro Sign",
       "\r": "Carriage Return",
       "\ufb33": "Hebrew Letter Dalet With Dagesh",
       "1": "One",
       "\ud83d\ude00": "Emoji: Grinning Face",
       "\u0080": "Control",
       "\u00f6": "Latin Small Letter O With Diaeresis"
     }

   Expected argument order after sorting property strings:
     "Carriage Return"   (U+000D)
     "One"               (U+0031)
     "Control"           (U+0080)
     "Latin Small Letter O With Diaeresis"  (U+00F6)
     "Euro Sign"         (U+20AC)
     "Emoji: Grinning Face"   (U+1F600)   <-- BEFORE U+FB33
     "Hebrew Letter Dalet With Dagesh"  (U+FB33)
```
Under UTF-8 / scalar order, U+FB33 (`EF AC B3`) sorts **before** U+1F600 (`F0 9F 98 80`) — the last two rows swap. So `JSONEncoder.sortedKeys` fails this exact RFC vector for any object containing both an astral-plane key and a U+E000–U+FFFF key.

Also non-conformant regardless of ordering: `swift-yyjson`'s `strcmp`-based sort truncates comparison at an embedded U+0000, which JSON permits in member names via `\u0000` [21].

**Severity: HIGH.** Anyone reaching for `.sortedKeys` to interoperate with a Rust `serde_json_canonicalizer` will match on pure-ASCII keys and silently diverge on non-ASCII keys — and, on Apple platforms with `compatibility1`, will diverge on *ASCII digit keys* too (`"2"` vs `"10"`).

---

### (d) NFC / canonical equivalence — the largest Swift↔Rust divergence

**Confirmed: Swift `String` equality, ordering, and hashing are all Unicode-canonical-equivalence-based.** This is not an implementation quirk; it is a documented, ABI-level guarantee.

Documentation, `stdlib/public/core/String.swift` [14] (also rendered at https://developer.apple.com/documentation/swift/string):

> Comparing strings for equality using the equal-to operator (`==`) or a relational operator (like `<` or `>=`) is always performed using Unicode canonical representation. As a result, different representations of a string compare as being equal.
>
> ```
> let cafe1 = "Cafe\u{301}"
> let cafe2 = "Café"
> print(cafe1 == cafe2)
> // Prints "true"
> ```
>
> The Unicode scalar value `"\u{301}"` modifies the preceding character to include an accent, so `"e\u{301}"` has the same canonical representation as the single Unicode scalar value `"é"`.
>
> Basic string operations are not sensitive to locale settings, ensuring that string comparisons and other operations always have a single, stable result, **allowing strings to be used as keys in `Dictionary` instances** and for other purposes.

**Implementation of `<`**, `stdlib/public/core/StringComparison.swift` [12] — this pins down *what order* is produced:
```swift
  internal func _slowCompare(
    with other: _StringGutsSlice,
    expecting: _StringComparisonResult
  ) -> Bool {
    var iter1 = Substring(self).unicodeScalars._internalNFC.makeIterator()
    var iter2 = Substring(other).unicodeScalars._internalNFC.makeIterator()
    ...
      if scalar1! < scalar2! {
        return expecting == .less
      }
```
with the fast path [12]:
```swift
  if _fastPath(bothNFC) {
    ...
    let cmp = unsafe _binaryCompare(utf8Left, utf8Right)
    return _lexicographicalCompare(cmp, 0, expecting: expecting)
```

⇒ **Swift `String <` = lexicographic order over NFC-normalized Unicode *scalars* (equivalently, NFC-normalized UTF-8 bytes).** That is *neither* raw-bytes (what Rust does) *nor* UTF-16 code units (what JCS requires). It is doubly wrong for JCS.

**Implementation of hashing**, `stdlib/public/core/StringHashable.swift` [13]:
```swift
extension String: Hashable {
  public func hash(into hasher: inout Hasher) {
    if _fastPath(self._guts.isNFCFastUTF8) {
      self._guts.withFastUTF8 { hasher.combine(bytes: UnsafeRawBufferPointer($0)) }
      hasher.combine(0xFF as UInt8) // terminator
    } else {
      _gutsSlice._normalizedHash(into: &hasher)
    }
  }
}
...
  internal func _normalizedHash(into hasher: inout Hasher) {
    if self.isNFCFastUTF8 { ... } else {
      _withNFCCodeUnits { hasher.combine($0) }
    }
```

⇒ **`Dictionary<String, _>` and `Set<String>` key on the NFC form.**

Note also that Swift does **not** normalize on storage — it validates UTF-8 on creation but preserves the original scalars (hence the `isNFC` flag and the whole `_stringCompareFastUTF8Abnormal` slow path) [12][25]. So `s.utf8` and `s.utf16` give the *original*, unnormalized code units while `==`/`<`/`hash` give the *normalized* ones. **This split is the trap**: a Swift canonicalizer that sorts `dict.keys` by `.utf16` (correct per JCS) is still operating on a key set that Foundation's `Dictionary` has already deduplicated by NFC.

#### What this means concretely

**1. Duplicate-key semantics differ from Rust. (Severity: HIGH — silent data loss / signature forgery surface.)**

Input JSON: `{"Cafe\u0301":1,"Café":2}` — two distinct member names (5 vs 4 UTF-16 code units; `RFC 8785 §3.2.3`: "The sorting process is applied to property name strings in their 'raw' (unescaped) form" [15]).

- Rust `serde_json::Map` (`BTreeMap<String,_>` or `IndexMap<String,_>`): `String: Ord/Hash` is **byte-wise**, so these are two distinct keys. Byte order: `Cafe\u{301}` = `43 61 66 65 CC 81`, `Café` = `43 61 66 C3 A9`; at index 3, `0x65 < 0xC3`, so `"Cafe\u{301}"` sorts first. Two members survive. *(Rust byte-comparison semantics is a well-known property of `std::string::String`; marked INFERRED-but-high-confidence — I did not re-read the Rust stdlib in this task, it was out of scope.)*
- Swift `[String: Any]` (used by `minacle/swift-jcs` [3], by `JSONSerialization`, and by `JSONEncoder`'s `[String: JSONEncoderValue]` [10]): **one member survives**, with last-writer or first-writer value depending on the parser. The canonical output, and therefore the SHA-256 and any signature over it, silently differs from Rust's.

**2. `String <` is not usable as the JCS comparator, ever.** `"K"` (U+004B) and `"K"` (U+212A KELVIN SIGN) compare **equal** in Swift because U+212A has a singleton canonical decomposition to U+004B [16]. Under JCS they are two distinct member names sorting far apart (0x004B vs 0x212A). Confirmed in the Swift bug tracker [16]:

> Swift is considering the following two Strings to be equivalent: 1) K 2) K But they are not. 1 is U+004B LATIN CAPITAL LETTER K and 2 is U+212A KELVIN SIGN. If you define them as NSStrings, they are not considered equivalent as expected.
> … "Unicode says these two characters *are* equivalent."

Swift is *canonical*-equivalence only, not compatibility: `"ﬃ"` (U+FB03) ≠ `"ffi"`, while `"caña"` NFC == `"caña"` NFD [24]. That narrows but does not eliminate the collision surface.

**3. The safe Swift construction.** To be Rust/JCS-byte-compatible, a Swift JCS implementation must:
   - never route member names through `Dictionary<String, _>`, `Set<String>`, `String ==`, or `String <`;
   - carry member names as `[UInt8]` (raw UTF-8) or `[UInt16]` (raw UTF-16) from the parser through to output;
   - sort with an explicit UTF-16 code-unit comparator over the **raw** (unnormalized) `.utf16` view;
   - detect duplicates by raw code-unit equality, not `String ==`.

   `minacle/swift-jcs` does the third of these correctly and fails the first, second, and fourth [3].

---

## Cross-cutting risk table for a Swift port of a Rust JCS canonicalizer

| # | Divergence | Where it bites | Severity |
|---|---|---|---|
| R1 | Swift `Double.description` uses exponential form above 2^53, ECMAScript above 1e21 | every double in (2^53, 1e21) | **critical** |
| R2 | Swift uses exponential below 10^-5, ECMAScript below 10^-7 | 1e-5 … 1e-6 decade | **critical** |
| R3 | Swift zero-pads exponents to 2 digits (`1e-07`) | all `|exp| < 10` | **critical** |
| R4 | Swift always appends `.0` to integral doubles | all integral doubles | high (mitigated by Foundation strip) |
| R5 | Swift emits `-0.0`; JCS requires `0` | negative zero | high |
| R6 | `Dictionary<String,_>` merges canonically-equivalent keys | any non-NFC-stable member name | **critical, silent** |
| R7 | `String <` orders by NFC-normalized scalars, not raw UTF-16 code units | non-ASCII / astral keys | **critical** |
| R8 | `JSONEncoder.sortedKeys` orders by UTF-8 (Linux) or locale NSString compare (Apple legacy) | non-ASCII keys; digit keys on Apple legacy | **critical** |
| R9 | `strcmp`-based sorts (swift-yyjson) truncate at embedded NUL | keys containing `\u0000` | medium |
| R10 | Swift emits `inf`/`nan`; JCS must error | non-finite doubles | medium |

---

## Sources

1. RFC 8785, *JSON Canonicalization Scheme (JCS)*, Appendix G "Open-Source Implementations" — https://www.rfc-editor.org/rfc/rfc8785.txt (also https://datatracker.ietf.org/doc/html/rfc8785)
2. cyberphone/json-canonicalization — https://github.com/cyberphone/json-canonicalization
3. minacle/swift-jcs — https://github.com/minacle/swift-jcs
4. SwiftPackageIndex/PackageList `packages.json` — https://github.com/SwiftPackageIndex/PackageList
5. swiftlang/swift, `stdlib/public/core/FloatingPointToString.swift` — https://github.com/swiftlang/swift/blob/main/stdlib/public/core/FloatingPointToString.swift
6. swiftlang/swift, `test/stdlib/PrintFloat.swift.gyb` — https://github.com/swiftlang/swift/blob/main/test/stdlib/PrintFloat.swift.gyb
7. ECMA-262 6th edition §7.1.12.1, *ToString Applied to the Number Type* — https://262.ecma-international.org/6.0/#sec-tostring-applied-to-the-number-type (current: https://tc39.es/ecma262/#sec-numeric-types-number-tostring)
8. RFC 8785 Appendix B, *Number Serialization Samples* — https://www.rfc-editor.org/rfc/rfc8785.txt
9. apple/swift-foundation, `Sources/FoundationEssentials/JSON/JSONWriter.swift` — https://github.com/apple/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONWriter.swift
10. apple/swift-foundation, `Sources/FoundationEssentials/JSON/JSONEncoder.swift` — https://github.com/apple/swift-foundation/blob/main/Sources/FoundationEssentials/JSON/JSONEncoder.swift
11. swiftlang/swift-corelibs-foundation, `Sources/Foundation/JSONSerialization.swift` — https://github.com/swiftlang/swift-corelibs-foundation/blob/main/Sources/Foundation/JSONSerialization.swift
12. swiftlang/swift, `stdlib/public/core/StringComparison.swift` — https://github.com/swiftlang/swift/blob/main/stdlib/public/core/StringComparison.swift
13. swiftlang/swift, `stdlib/public/core/StringHashable.swift` — https://github.com/swiftlang/swift/blob/main/stdlib/public/core/StringHashable.swift
14. swiftlang/swift, `stdlib/public/core/String.swift` — https://github.com/swiftlang/swift/blob/main/stdlib/public/core/String.swift ; rendered: https://developer.apple.com/documentation/swift/string
15. RFC 8785 §3.2.3, *Sorting of Object Properties* — https://www.rfc-editor.org/rfc/rfc8785.txt
16. Swift issue #46339 (SR-3754), "Two Strings are considered equivalent when they are not" — https://github.com/apple/swift/issues/46339
17. Swift Forums, "Double to String conversion implementation" (Nov 2025) — https://forums.swift.org/t/double-to-string-conversion-implementation/83335
18. swiftlang/swift PR #15474, "SR-106: New floating-point `description` implementation" — https://github.com/swiftlang/swift/pull/15474
19. apple/swift PR #35299, "SwiftDtoa v2: Better, Smaller, Faster floating-point formatting" — https://github.com/apple/swift/pull/35299
20. amosavian/JWSETKit, `Sources/JWSETKit/Cryptography/Keys.swift` — https://github.com/amosavian/JWSETKit/blob/main/Sources/JWSETKit/Cryptography/Keys.swift
21. mattt/swift-yyjson PR #14, "Add sortedKeys option for JSON encoding" — https://github.com/mattt/swift-yyjson/pull/14
22. apple/swift-foundation issue #284 — https://github.com/apple/swift-foundation/issues/284
23. proxyco/swift-jose — https://github.com/proxyco/swift-jose
24. Swift Forums, "Swift string comparison doesn't consider ligatures equivalent to their components" — https://forums.swift.org/t/swift-string-comparison-doesnt-consider-ligatures-equivalent-to-their-components/66665
25. Swift.org blog, "UTF-8 String" — https://www.swift.org/blog/utf8-string/
26. swiftlang/swift-evolution SE-0363, "Unicode for String Processing" (background on canonical-equivalence default view) — https://github.com/swiftlang/swift-evolution/blob/main/proposals/0363-unicode-for-string-processing.md
27. UAX #15, *Unicode Normalization Forms* — https://unicode.org/reports/tr15/
28. RFC 7638, *JSON Web Key (JWK) Thumbprint* — https://www.rfc-editor.org/rfc/rfc7638.html

---

## Coverage Status

**Checked directly (source cloned/read or spec text read verbatim):**
- `done` — RFC 8785 full text: Appendix G implementation list, Appendix B number table, §3.2.2.3, §3.2.3 sorting rules + conformance vector. Downloaded `rfc8785.txt` and grepped; `swift` appears **0 times**.
- `done` — `cyberphone/json-canonicalization` full repo tree + README implementations table. No `swift/` directory.
- `done` — `minacle/swift-jcs` at `1e69bef`: all three source files and `Package.swift` read in full.
- `done` — `apple/swift-foundation` @ `f0442fb3` (2026-08-05): `JSONWriter.swift` sortedKeys block, `JSONEncoder.swift` OutputFormatting + float encoding.
- `done` — `swiftlang/swift-corelibs-foundation` main (2026-07-22): `JSONSerialization.swift` sortedKeys block.
- `done` — `swiftlang/swift` main: `FloatingPointToString.swift` (design comment, `_finishFormatting`, `_Float64ToASCII` `forceExponential`), `StringComparison.swift`, `StringHashable.swift`, `String.swift` doc comment, `test/stdlib/PrintFloat.swift.gyb`.
- `done` — ECMA-262 6th ed. §7.1.12.1 full algorithm text + NOTE 1 + NOTE 2.
- `done` — GitHub repository search across 7 query variants with `language:swift`; `PackageList/packages.json` grep.
- `done` — `JWSETKit` `jwkThumbprint` implementation.

**Uncertain / not directly verified:**
- **Swift runtime execution.** No Swift toolchain, container runtime, or `download.swift.org` reachable in this sandbox (`which swift` → not found; `which docker podman` → not found). Every Swift output string I report is taken from the **official Swift test suite's own `expectDescription` golden values** [6] or derived from the `_finishFormatting` branch conditions [5]. Rows in the D2/D3 tables for values *not* literally present in the test suite (`2.9514790517935283e+20`, `9.999999999999997e+20`, `1e-06`, `-3.3333333333333333e-06`, `9.999999999999997e-07`) are **derived from code + test-suite pattern, not executed**. They should be confirmed with a one-line Swift script before being relied on.
- **`JSONEncoder.compatibility1` definition** is not in the open-source tree (Apple-internal FOUNDATION_FRAMEWORK). Its exact activation condition is **INFERRED** to be a linked-on-or-after binary-compatibility check.
- **Closed-source Apple Foundation `JSONEncoder`** (Darwin, pre-swift-foundation-adoption OS versions) — ordering not directly readable; the `compatibility1` branch in swift-foundation is my evidence for what it did.
- **Rust `String: Ord` byte semantics** — asserted from general knowledge, not re-verified in this task (out of assigned scope). Flagged INFERRED where used.
- `ai-university-aiu/causalontology` Swift binding and other vendored in-repo JCS fragments — **not read**, only search snippets seen. Deliberately excluded from the implementation table.
- `swiftpackageindex.com` returns HTTP 403 to non-browser clients; SPI coverage was verified via the underlying `PackageList/packages.json` instead.

**Not attempted:**
- Running RFC 8785 `testdata/` vectors (`arrays.json`, `french.json`, `structures.json`, `unicode.json`, `values.json`, `weird.json` — present in the cloned repo at `/tmp/pi-github-repos/cyberphone/json-canonicalization/testdata/`) against `minacle/swift-jcs`. This is the single highest-value follow-up and requires a Swift 6.3 toolchain.
