# Rust implementations of RFC 8785 (JSON Canonicalization Scheme) — evidence report

Date of investigation: **2026-08-06**. Toolchain used for empirical verification: `rustc/cargo 1.97.1 (c980f4866 2026-06-30)`.

Everything below marked **VERIFIED** was reproduced by downloading the exact published `.crate` tarball
from crates.io, reading the source, and compiling + running it locally. Reproduction harnesses:
`/tmp/jcsdiff` (serde_jcs 0.2 / serde_json_canonicalizer / json-canon / canon-json),
`/tmp/vrtest` (vr-jcs), `/tmp/sj010` (serde_jcs 0.1.0), `/tmp/apbtest` (arbitrary_precision feature-unification).
Anything I did not execute or read directly is labelled **INFERRED** or **NOT CHECKED**.

---

## Evidence table

| # | Source | URL | Key claim | Type | Confidence |
|---|--------|-----|-----------|------|------------|
| 1 | RFC 8785 §3.2.2.3 | https://www.rfc-editor.org/rfc/rfc8785.txt | "occurrences of NaN or Infinity MUST cause a compliant JCS implementation to terminate with an appropriate error" | primary | high |
| 2 | RFC 8785 §3.2.3 | https://www.rfc-editor.org/rfc/rfc8785.txt | "Property name strings to be sorted are formatted as arrays of UTF-16 [UNICODE] code units"; sorting applied to "raw" (unescaped) form | primary | high |
| 3 | crates.io `serde_jcs` | https://crates.io/crates/serde_jcs | 0.2.0, released 2026-03-25; 0.1.0 released 2020-11-13; 2,085,675 total downloads; 102 dependents | primary | high |
| 4 | `serde_jcs` 0.2.0 source | https://docs.rs/crate/serde_jcs/0.2.0/source/src/lib.rs | `struct Utf16Key { tag: Vec<u16>, ... }`, `.encode_utf16().collect()`, `BTreeMap<Utf16Key, Vec<u8>>`, `ryu_js::Buffer::format_finite` | primary | high |
| 5 | `serde_jcs` 0.1.0 source | https://docs.rs/crate/serde_jcs/0.1.0/source/src/entry.rs | `pub object: BTreeMap<Vec<u8>, Vec<u8>>` — escaped UTF-8 byte sort | primary | high |
| 6 | serde_jcs issue #1 | https://github.com/l1h3r/serde_jcs/issues/1 | "Sorting properties as unescaped UTF-16" — opened 2020-12-29, closed 2026-03-25; includes exact wrong-vs-expected output | primary | high |
| 7 | serde_jcs issue #3 | https://github.com/l1h3r/serde_jcs/issues/3 | OPEN since 2021-09-09: "Encoding numbers does not follow specification"; "the official go implementation reduces the precision when encoding `uint64`, while serde_jcs does not, leading to a different result" | primary | high |
| 8 | serde_jcs issue #2 | https://github.com/l1h3r/serde_jcs/issues/2 | "Test with test data referenced from the specification" — closed 2026-03-25 | primary | high |
| 9 | serde_jcs commit log | https://github.com/l1h3r/serde_jcs/commits/main | 2026-03-25 commits: "Return errors for invalid floats", "Add RFC test data", "Internal refactor" | primary | high |
| 10 | crates.io `json-canon` | https://crates.io/crates/json-canon | 0.1.3, last release 2023-05-13; 479,848 downloads | primary | high |
| 11 | `json-canon` ser.rs | https://docs.rs/crate/json-canon/0.1.3/source/src/ser.rs | `FpCategory::Zero => writer.write_all(b"0")`; NaN/Infinite → `Err(...)`; `ryu_js::Buffer::new().format_finite`; MAX_SAFE_INTEGER guards | primary | high |
| 12 | `json-canon` object.rs | https://docs.rs/crate/json-canon/0.1.3/source/src/object.rs | `key_orig.encode_utf16()`; `entries.sort_by(|a, b| a.cmpable().cmp(b.cmpable()))` — no dedup | primary | high |
| 13 | json-canon issue #6 | https://github.com/ahdinosaur/json-canon/issues/6 | "large integers (beyond JavaScript's Number.MAX_SAFE_INTEGER) should error" — closed 2023-05-13 | primary | high |
| 14 | json-canon issue #5 | https://github.com/ahdinosaur/json-canon/issues/5 | "number keys are not properly handled" — closed 2023-05-13, includes failing test output | primary | high |
| 15 | crates.io `serde_json_canonicalizer` | https://crates.io/crates/serde_json_canonicalizer | 0.3.2, released 2026-02-03; 4,269,956 downloads; 59 dependents; dep `ryu-js ^1.0.1` | primary | high |
| 16 | `serde_json_canonicalizer` jcs.rs | https://docs.rs/crate/serde_json_canonicalizer/0.3.2/source/src/jcs.rs | `sorting_key: Vec<u16>` via `.encode_utf16()`; `type JsonObject = BTreeSet<JsonProperty>`; `ryu_js::Buffer` | primary | high |
| 17 | sjc issue #5 | https://github.com/evik42/serde-json-canonicalizer/issues/5 | "Numbers are output with ff as a prefix" — will output `{"number":ff300}` if serde_json has arbitrary_precision turned on; closed 2025-08-10 | primary | high |
| 18 | sjc issue #9 | https://github.com/evik42/serde-json-canonicalizer/issues/9 | "Object properties" — asks whether sorting holds under `preserve_order`; closed 2026-02-11 | primary | high |
| 19 | crates.io `canon-json` | https://crates.io/crates/canon-json | 0.2.1, released 2025-06-23; 283,378 downloads; repo redirects to bootc-dev/canon-json-rs | primary | high |
| 20 | `canon-json` floatformat.rs | https://docs.rs/crate/canon-json/0.2.1/source/src/floatformat.rs | Custom formatter: `num.to_string()` / `format!("{:e}", num)` + manual `+` insertion; no ryu dependency | primary | high |
| 21 | `canon-json` lib.rs | https://docs.rs/crate/canon-json/0.2.1/source/src/lib.rs | `struct ObjectKey(Vec<u16>)`, `s.encode_utf16().collect()`, `BTreeMap<ObjectKey, Vec<u8>>`; `write_number_str` passes through unchanged | primary | high |
| 22 | canon-json-rs issue #16 | https://github.com/bootc-dev/canon-json-rs/issues/16 | OPEN: "Fails to compile when `arbitrary_precision` and `raw_value` are enabled for `serde_json`" (E0275 recursion overflow) | primary | high |
| 23 | crates.io `vr-jcs` | https://crates.io/crates/vr-jcs | 0.4.1, released 2026-05-11; 820 downloads; deps include `zmij ^1`, serde_json with `arbitrary_precision`+`preserve_order`+`unbounded_depth` | primary | high |
| 24 | `vr-jcs` number.rs | https://docs.rs/crate/vr-jcs/0.4.1/source/src/number.rs | `zmij::Buffer::new()` + hand-written `render_ecmascript_number`; `if value == 0.0 { return Ok("0") }`; `ensure_exact_binary64_integer` errors | primary | high |
| 25 | `vr-jcs` canonicalize.rs | https://docs.rs/crate/vr-jcs/0.4.1/source/src/canonicalize.rs | `fn cmp_utf16(left,right) { left.encode_utf16().cmp(right.encode_utf16()) }` | primary | high |
| 26 | `vr-jcs` strict_parse.rs | https://docs.rs/crate/vr-jcs/0.4.1/source/src/strict_parse.rs | Rejects duplicate property names and Unicode noncharacters `U+FDD0..=U+FDEF`, `U+xFFFE`, `U+xFFFF` | primary | high |
| 27 | crates.io `jcs-canonicalize` | https://crates.io/crates/jcs-canonicalize | 0.2.1, released 2026-05-21; 388 downloads; thin wrapper over `serde_jcs ^0.2` | primary | high |
| 28 | `jcs-canonicalize` corpus test | https://docs.rs/crate/jcs-canonicalize/0.2.1/source/tests/cyberphone_corpus.rs | Vendors cyberphone corpus (arrays/french/structures/unicode/values/weird) and asserts byte-identical output + idempotence | primary | high |
| 29 | cyberphone/json-canonicalization | https://github.com/cyberphone/json-canonicalization | Reference implementation + testdata referenced by RFC 8785 Appendix I | primary | high |
| 30 | cyberphone issue #20 | https://github.com/cyberphone/json-canonicalization/issues/20 | OPEN "Support for uint64"; Rundgren: "This is by design because JCS (RFC 8785) requires data to confirm to the JSON subset specified by JavaScript (aka I-JSON) … Numbers outside of float64 must thus be put in quotes"; explicitly names serde_jcs as non-conforming | primary | high |
| 31 | `ryu-js` README | https://github.com/boa-dev/ryu-js | "Ryū-js is a fork of the ryu crate adjusted to comply to the ECMAScript number-to-string algorithm" | primary | high |
| 32 | crates.io `ryu-js` | https://crates.io/crates/ryu-js | latest 1.0.3 (2026-07-10); 0.2.2 (2021-12-16) is the version serde_jcs and json-canon pin | primary | high |
| 33 | crates.io `zmij` | https://crates.io/crates/zmij | 1.0.23; dtolnay; "double-to-string conversion algorithm based on Schubfach and xjb" | primary | high |
| 34 | serde_jcs per-version downloads | https://crates.io/api/v1/crates/serde_jcs/downloads | Last 90 days: 0.2.0 = 750,911; **0.1.0 = 569,908** (still ~43% of live traffic on the broken version) | primary | high |
| 35 | sjc README | https://github.com/evik42/serde-json-canonicalizer#readme | Documents that arbitrary-precision numbers are converted to doubles and "the arbitrary precision is lost" | primary | high |

---

## 1. Crate inventory (a)

### 1.1 General-purpose JCS crates

| Crate | Latest | Repo | Last release | Downloads (total) | Dependents |
|---|---|---|---|---|---|
| `serde_jcs` | **0.2.0** | https://github.com/l1h3r/serde_jcs | **2026-03-25** | 2,085,675 | 102 |
| `serde_json_canonicalizer` | **0.3.2** | https://github.com/evik42/serde-json-canonicalizer | **2026-02-03** | 4,269,956 | 59 |
| `json-canon` | **0.1.3** | https://github.com/ahdinosaur/json-canon | **2023-05-13** | 479,848 | — |
| `canon-json` | **0.2.1** | https://github.com/containers/canon-json-rs → redirects to https://github.com/bootc-dev/canon-json-rs | **2025-06-23** | 283,378 | — |
| `vr-jcs` | **0.4.1** | https://github.com/VertRule/vr-jcs | **2026-05-11** | 820 | 2 |
| `jcs-canonicalize` | **0.2.1** | https://github.com/arcanesys/jcs-canonicalize | **2026-05-21** | 388 | 0 |

Sources: [3], [15], [10], [19], [23], [27]. All version/date/download figures come from the crates.io API
(`https://crates.io/api/v1/crates/<name>`), retrieved 2026-08-06.

`serde_jcs` version history: 0.1.0 (2020-11-13) → 0.2.0 (2026-03-25). **There were no intermediate releases;
0.1.0 was the only published version for 5 years and 4 months** [3].

### 1.2 Other crates on crates.io that claim RFC 8785 (checked deps only, source NOT read)

These surfaced from `crates.io/api/v1/crates?q=jcs` and `?q=rfc8785`. I checked their crates.io metadata and
declared dependencies but did **not** read their source, so no behavioural claims are made about them.

| Crate | Latest | Repo | Updated | Normal deps (from crates.io API) |
|---|---|---|---|---|
| `acdp-jcs` | 0.8.1 | https://github.com/agentcontextdistributionprotocol/acdp-rs | 2026-07-10 | acdp-primitives, serde, serde_json — **no ryu-js**, own float path NOT CHECKED |
| `boundary-compiler` | 0.1.1 | https://github.com/RecursiveIntell/boundary-compiler | 2026-07-15 | blake3, **ryu-js**, serde, serde_json, sha2, thiserror |
| `reallyme-codec-jcs` | 0.2.1 | https://github.com/reallyme/codec | 2026-07-27 | itoa, **ryu-js**, serde, serde_json, thiserror, zeroize |
| `canaad-core` / `canaad-cli` | 2.0.0 | https://github.com/gnufood/canaad | 2026-04-29 | **wraps `serde_json_canonicalizer`** |
| `warpin-integrity` | 0.2.6 | https://github.com/time-origin/warpin-rs-common | 2026-07-16 | **wraps `serde_jcs`** |
| `assay-canonical` | 3.38.0 | https://github.com/Rul1an/assay | 2026-08-04 | **wraps `serde_jcs`** |
| `vauban-x402-jcs-conformance` | 0.1.1 | https://github.com/vauban-org/vauban-zkpay | 2026-05-29 | **wraps `serde_jcs`** |

**INFERRED:** the four `serde_jcs`-wrapping crates inherit whatever `serde_jcs` version resolves under their
`^0.2`/`^0.1` requirement, and therefore inherit every property described in §2–§5 below. I did not read their code.

`jcs-canonicalize` 0.2.1 is a **thin wrapper**, verified by reading its full 64-line `src/lib.rs`:

```rust
pub fn canonicalize(input: &str) -> Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(input).context("input is not valid JSON")?;
    serde_jcs::to_string(&value).context("JCS canonicalization failed")
}
```
— https://docs.rs/crate/jcs-canonicalize/0.2.1/source/src/lib.rs [27]

Its dependency is `serde_jcs = "0.2"`, so it gets 0.2.0 behaviour.

---

## 2. Float-formatting backend, and why it matters (b)

### 2.1 Backend per crate (read directly from source)

| Crate | Backend | Exact source line |
|---|---|---|
| `serde_jcs` 0.2.0 | **ryu-js `^0.2`** | `let mut buffer: Buffer = Buffer::new();` … `writer.write_all(buffer.format_finite(value).as_bytes())` [4] |
| `serde_jcs` 0.1.0 | **ryu-js `^0.2`** (same) | dep declared in Cargo.toml [5] |
| `serde_json_canonicalizer` 0.3.2 | **ryu-js `^1.0.1`** | `let mut buffer = ryu_js::Buffer::new(); let s = buffer.format_finite(value);` [16] |
| `json-canon` 0.1.3 | **ryu-js `^0.2.2`** | `writer.write_all(ryu_js::Buffer::new().format_finite(value).as_bytes())` [11] |
| `canon-json` 0.2.1 | **CUSTOM** — Rust `Display` + `LowerExp` | `num.to_string()` for `(1e-6..1e21)`, else `format!("{:e}", num)` with manual `+` insertion [20] |
| `vr-jcs` 0.4.1 | **`zmij` `^1`** (dtolnay's Schubfach implementation) + hand-written ES rendering layer | `let mut buffer = zmij::Buffer::new(); let shortest = buffer.format_finite(value);` then `render_ecmascript_number(&digits, exponent)` [24] |

### 2.2 Why the backend matters — measured ryu vs ryu-js vs Rust `Display`

**VERIFIED** by running `ryu::Buffer::format`, `ryu_js::Buffer::format`, and `format!("{v}")` on the same f64 values
(`/tmp/jcsdiff/src/main.rs`, built against `ryu 1` and `ryu-js 1`):

| f64 value | Rust `Display` (`{}`) | `ryu` (shortest, plain Rust) | `ryu-js` (ECMAScript) |
|---|---|---|---|
| `1e21` | `1000000000000000000000` | `1e21` | **`1e+21`** |
| `1e20` | `100000000000000000000` | `1e20` | **`100000000000000000000`** |
| `1e-7` | `0.0000001` | `1e-7` | `1e-7` |
| `1e-6` | `0.000001` | `1e-6` | **`0.000001`** |
| `5e-324` | 324-digit literal `0.000…005` | `5e-324` | `5e-324` |
| `-0.0` | `-0` | **`-0.0`** | **`0`** |
| `9007199254740992.0` | `9007199254740992` | **`9007199254740992.0`** | `9007199254740992` |
| `295147905179352830000.0` | `295147905179352830000` | **`2.9514790517935283e20`** | `295147905179352830000` |
| `f64::MAX` | 309-digit literal | `1.7976931348623157e308` | **`1.7976931348623157e+308`** |

So plain `ryu` is wrong for JCS on **five of the parent's six named values** (`1e21`, `1e-7`→ok, `1e-6`, `5e-324`→ok,
`9007199254740992`, `295147905179352830000`) and on `-0.0`. Plain Rust `Display` is wrong on `1e21`, `5e-324`,
`f64::MAX`, and `-0.0`. `ryu-js` matches ECMAScript on all of them, which is why it is the correct backend and
why `ryu-js` describes itself as "a fork of the ryu crate adjusted to comply to the ECMAScript number-to-string
algorithm" [31].

Three specific ryu-vs-ryu-js failure modes visible above:
- **Exponent sign.** `ryu` emits `1e21` / `1.7976931348623157e308`; ECMAScript requires `1e+21` / `…e+308`.
- **Exponent threshold.** `ryu` switches to exponential form far earlier than ECMAScript (`1e20` → `1e20`,
  `1e-6` → `1e-6`, `2.9514790517935283e20`); ECMAScript uses fixed notation across `[1e-6, 1e21)`.
- **Trailing `.0` and `-0.0`.** `ryu` always emits a fraction (`9007199254740992.0`, `-0.0`); ECMAScript emits
  `9007199254740992` and `0`.

### 2.3 Measured number output — all six crates agree except on integer width

**VERIFIED.** Inputs parsed from JSON text into `serde_json::Value`, then canonicalized:

| JSON input | serde_jcs 0.2.0 | serde_json_canonicalizer | json-canon | canon-json | vr-jcs |
|---|---|---|---|---|---|
| `1e21` | `1e+21` | `1e+21` | `1e+21` | `1e+21` | `1e+21` |
| `1e20` | `100000000000000000000` | same | same | same | same |
| `1e-6` | `0.000001` | same | same | same | same |
| `1e-7` | `1e-7` | same | same | same | same |
| `5e-324` | `5e-324` | same | same | same | same |
| `4.5e-324` | `5e-324` | same | same | same | same |
| `-0` | `0` | `0` | `0` | `0` | `0` |
| `1.7976931348623157e308` | `1.7976931348623157e+308` | same | same | same | same |
| `2.225073858507201e-308` | `2.225073858507201e-308` | same | same | same | same |
| `295147905179352830000` | `295147905179352830000` | same | same | same | same |
| `1.50` | `1.5` | same | same | same | same |
| `123456789012345678901234567890` | `1.2345678901234568e+29` | same | same | same | same |
| **`9007199254740992`** (2^53) | `9007199254740992` | `9007199254740992` | **`ERR: u64 must be less than JSON max safe integer`** | `9007199254740992` | `9007199254740992` |
| **`9007199254740993`** (2^53+1) | **`9007199254740992`** (rounds) | **`9007199254740992`** (rounds) | **ERR** | **`9007199254740993`** (passes through!) | **`ERR: integer … is not exactly representable as an IEEE 754 double`** |
| **`18446744073709551615`** (u64::MAX) | `18446744073709552000` | `18446744073709552000` | **ERR** | `18446744073709551615` | **ERR** |
| `1e400` | rejected by `serde_json` parser: `number out of range` | same | same | same | same |

This is a **four-way interop split on integers outside the IEEE-754 exact range**:
- `serde_jcs`, `serde_json_canonicalizer` — silently round through f64 (matches the Go/JS reference implementations).
- `canon-json` — passes the exact integer through unchanged. **Diverges from the reference implementation.**
  (canon-json uses `wrapper!(write_u64, u64)` delegating to `CompactFormatter`, i.e. no f64 round-trip [21].)
- `json-canon` — errors, and errors even at `2^53` itself, which the reference accepts (its guard is
  `value > MAX_SAFE_INTEGER` where `MAX_SAFE_INTEGER = 9_007_199_254_740_991 = 2^53 - 1` [11]).
- `vr-jcs` — errors, but its check is `bit_len <= 53 || value.trailing_zeros() >= bit_len - 53`, so it accepts
  `2^53` and any larger integer that is exactly representable [24].

Anders Rundgren's position (reference-implementation author) on cyberphone issue #20 [30]:

> "This is by design because JCS (RFC 8785) requires data to confirm to the JSON subset specified by JavaScript
> (aka I-JSON). No IETF standards based on JSON does (AFAIK...) go outside of this limit. Numbers outside of
> float64 must thus be put in quotes."

and, in the same thread, a comment from `matthiasgeihs` explicitly naming this crate:

> "Also note that there exists an unofficial rust implementation, [serde_jcs](https://crates.io/crates/serde_jcs),
> which des not seem to conform with the standard as it encodes `uint64` without losing precision."

That was true of serde_jcs 0.1.0 and is now **fixed in 0.2.0** — **VERIFIED**: 0.1.0 emits `9007199254740993`,
0.2.0 emits `9007199254740992`. The tracking issue [7] is nonetheless **still open**.

---

## 3. Object-key sort order (c)

**All six library crates sort by UTF-16 code units. There is no UTF-8-byte or Unicode-scalar sorter among the
current published versions.** Source lines:

- `serde_jcs` 0.2.0 [4]:
  ```rust
  struct Utf16Key { tag: Vec<u16>, key: Vec<u8> }
  let tag: Vec<u16> = from_slice::<Value>(&key)?.as_str().ok_or_else(invalid_key)?.encode_utf16().collect();
  impl Ord for Utf16Key { fn cmp(&self, other: &Self) -> Ordering { self.tag.cmp(&other.tag) } }
  ...
  object: BTreeMap<Utf16Key, Vec<u8>>,
  ```
  Note it re-parses the *serialized* key back through `serde_json::from_slice::<Value>` to get the **unescaped**
  string before `encode_utf16` — that satisfies RFC 8785 §3.2.3's "raw (unescaped) form" requirement [2].

- `serde_json_canonicalizer` 0.3.2 [16]:
  ```rust
  // Go through deserialization again to process escape sequences in the key
  // "\\a" should be processed as '\a' for sorting
  let sorting_key_as_value = serde_json::from_slice::<serde_json::Value>(&key)?;
  let sorting_key: Vec<u16> = sorting_key_as_value.as_str()...encode_utf16().collect();
  ...
  type JsonObject = BTreeSet<JsonProperty>;
  ```

- `json-canon` 0.1.3 [12]:
  ```rust
  pub(crate) fn cmpable<'a>(&'a self) -> impl Iterator<Item = impl Ord + 'a> {
      let key_orig = unsafe { from_utf8_unchecked(self.key_bytes.as_slice()) };
      key_orig.encode_utf16()
  }
  ...
  entries.sort_by(|a, b| a.cmpable().cmp(b.cmpable()));
  ```
  (`key_bytes` is a parallel buffer holding the *unescaped* key — `write_char_escape` writes the raw byte to
  `key_bytes` and the escape sequence to the output scope [11].)

- `canon-json` 0.2.1 [21]:
  ```rust
  /// https://www.rfc-editor.org/rfc/rfc8785#name-sorting-of-object-properties
  #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
  struct ObjectKey(Vec<u16>);
  fn new_from_str(s: &str) -> Self { Self(s.encode_utf16().collect()) }
  ...
  obj: BTreeMap<ObjectKey, Vec<u8>>,
  ```

- `vr-jcs` 0.4.1 [25]:
  ```rust
  fn cmp_utf16(left: &str, right: &str) -> Ordering {
      left.encode_utf16().cmp(right.encode_utf16())
  }
  ...
  entries.sort_by(|(left, _), (right, _)| cmp_utf16(left, right));
  ```

### 3.1 The discriminating test, run for real

The only place UTF-16 order and UTF-8/scalar order disagree is non-BMP (`U+10000..U+10FFFF`) vs `U+E000..U+FFFF`:
`U+20BB7` (𠮷) is UTF-8 `F0 A0 AE B7` / UTF-16 `D842 DFB7`; `U+FFA5` (ﾥ) is UTF-8 `EF BE A5` / UTF-16 `FFA5`.
UTF-8 byte order puts ﾥ first; UTF-16 code-unit order puts 𠮷 first.

**VERIFIED** — input `{"\u{FFA5}":1,"\u{20BB7}":2}`:

| Crate | Output | Verdict |
|---|---|---|
| `serde_jcs` **0.2.0** | `{"𠮷":2,"ﾥ":1}` | **correct (UTF-16)** |
| `serde_json_canonicalizer` 0.3.2 | `{"𠮷":2,"ﾥ":1}` | correct |
| `json-canon` 0.1.3 | `{"𠮷":2,"ﾥ":1}` | correct |
| `canon-json` 0.2.1 | `{"𠮷":2,"ﾥ":1}` | correct |
| `vr-jcs` 0.4.1 | `{"𠮷":2,"ﾥ":1}` | correct |
| `serde_jcs` **0.1.0** | **`{"ﾥ":1,"𠮷":2}`** | **WRONG (UTF-8 byte order)** |

Same result for `U+10000` vs `U+E000` (all current crates put `𐀀` before ``).

### 3.2 serde_jcs 0.1.0 — the historical bug, still ~43% of live traffic

`serde_jcs` 0.1.0 source, `src/entry.rs` line 5 [5]:
```rust
pub object: BTreeMap<Vec<u8>, Vec<u8>>,
```
The key is the **already-serialized, still-escaped, quote-included** byte string, sorted as UTF-8 bytes.
This produces two separate defects, both **VERIFIED** by running the cyberphone `weird.json` input through 0.1.0:

```
serde_jcs 0.1.0 : {"1":"One","</script>":"Browser Challenge","\n":"Newline","\r":"Carriage Return",
                   "\u0080":"Control\u007f","ö":"...","€":"...","דּ":"...","😂":"Smiley"}
RFC-correct     : {"\n":"Newline","\r":"Carriage Return","1":"One","</script>":"Browser Challenge",
                   "\u0080":"Control\u007f","ö":"...","€":"...","😂":"Smiley","דּ":"..."}
```
1. **Escaped-form sorting.** `"\n"` and `"\r"` sort *after* `"1"` and `"</script>"` because they are compared as
   the two-byte escape `5C 6E` (`\n`), and `0x5C > 0x3C ('<') > 0x31 ('1')`. RFC 8785 §3.2.3 requires comparing
   the *raw* U+000A [2].
2. **UTF-8 rather than UTF-16.** `😂` (U+1F602) sorts *after* `דּ` (U+FB33) because UTF-8 `F0 9F…` > `EF AC…`;
   UTF-16 requires `D83D…` < `FB33`.

This exactly reproduces the wrong-vs-expected output pasted in issue #1 on 2020-12-29 [6], which sat open for
**5 years and 3 months** and was closed only on 2026-03-25 when 0.2.0 shipped. The issue author independently
confirmed the expected form against the `json-canonicalize` npm module and against the W3C JSON-LD
`toRdf-manifest#tjs13` test case.

**Live-traffic risk (VERIFIED from crates.io download API [34]):** in the last 90 days `serde_jcs` 0.1.0 still
received **569,908 downloads** vs 750,911 for 0.2.0. Any consumer with `serde_jcs = "0.1"` in `Cargo.toml`
(as opposed to `"0.2"`) is *still* pinned to the UTF-8-sorting version, because 0.2.0 is a semver-incompatible
bump. This is the single most consequential finding in this report.

---

## 4. Negative zero, NaN/Infinity, lone surrogates (d)

### 4.1 Negative zero — all crates correct, but by three different mechanisms

**VERIFIED**: every crate emits `0` for both the JSON literal `-0` and the native Rust value `-0.0f64`.

| Crate | Mechanism |
|---|---|
| `serde_jcs`, `serde_json_canonicalizer` | delegated to `ryu_js::format_finite`, which returns `"0"` for `-0.0` (measured directly: `ryu-js` → `0`, plain `ryu` → `-0.0`) |
| `json-canon` | explicit: `FpCategory::Zero => writer.write_all(b"0")` [11] |
| `canon-json` | explicit: `if ieee_f64 == 0.0 { return Ok("0".to_string()); }` with comment *"Special case: Eliminate `-0` as mandated by the ES6/JCS specifications"* [20] |
| `vr-jcs` | explicit: `if value == 0.0 { return Ok("0".to_string()); }` [24] |

`serde_jcs` 0.2.0 has an explicit RFC Appendix B regression test [https://docs.rs/crate/serde_jcs/0.2.0/source/tests/basic.rs]:
```rust
assert_json_number(0x8000_0000_0000_0000, "0"); // Minus zero
assert_json_number(0x0000_0000_0000_0001, "5e-324"); // Min pos number
assert_json_number(0x4430_0000_0000_0000, "295147905179352830000"); // ~2**68
assert_json_number_err(0x7fff_ffff_ffff_ffff); // NaN
assert_json_number_err(0x7ff0_0000_0000_0000); // Infinity
```

### 4.2 NaN / Infinity — a real split, and two crates are non-compliant

RFC 8785 §3.2.2.3 [1]:
> "Note: Since Not a Number (NaN) and Infinity are not permitted in JSON, occurrences of NaN or Infinity **MUST**
> cause a compliant JCS implementation to terminate with an appropriate error."

NaN/Infinity **cannot** arrive through the JSON-text path (`serde_json`'s parser rejects `NaN`/`Infinity` tokens —
**VERIFIED**: `expected value at line 1 column 6`). The split appears on the **native Rust value path**
(`to_string(&f64::NAN)`, or any struct with an `f64` field holding NaN):

| Crate | `to_string(&f64::NAN)` — **VERIFIED** |
|---|---|
| `serde_jcs` 0.2.0 | `Err("invalid float value")` ✅ |
| `serde_json_canonicalizer` 0.3.2 | `Err("NaN and +/-Infinity are not permitted in JSON")` ✅ |
| **`json-canon` 0.1.3** | **`Ok("null")`** ❌ |
| **`canon-json` 0.2.1** | **`Ok("null")`** ❌ |
| `serde_jcs` **0.1.0** | **`Ok("null")`** ❌ |
| `vr-jcs` 0.4.1 | n/a (byte-slice API only; parser rejects) |

Root cause (**VERIFIED** by source reading, and consistent with the observed behaviour): `serde_json`'s
`Serializer::serialize_f64` intercepts non-finite values and calls `Formatter::write_null` *before* the custom
`Formatter::write_f64` is ever reached. `json-canon` and `canon-json` only override the `Formatter`, so their
NaN branches (`FpCategory::Nan => Err(...)` [11], `INVALID_PATTERN` check [20]) are **dead code on this path**.
`serde_jcs` 0.2.0 and `serde_json_canonicalizer` fix this by wrapping the *`Serializer`* and checking
`value.is_finite()` before delegating [4], [16] — for `serde_jcs` this was the 2026-03-25 commit
*"Return errors for invalid floats"* [9].

This is a genuine RFC 8785 §3.2.2.3 MUST violation in `json-canon` 0.1.3 and `canon-json` 0.2.1 for the
native-Rust-value entry point. (It does happen to match `JSON.stringify(NaN) === "null"`, but the RFC's
NaN note is unambiguous.) **No open issue exists for this on either repo** — I read the full issue+PR lists
for both and found nothing matching.

### 4.3 Lone surrogates / unpaired UTF-16 — confirmed rejected at parse, never replaced

The parent's hypothesis is **CONFIRMED VERIFIED**:

```
char::from_u32(0xD800)                  = None
String::from_utf8(vec![0xED,0xA0,0x80]) = Err("invalid utf-8 sequence of 1 bytes from index 0")
```
Rust's `char` is a Unicode *scalar value* by definition, so `String` provably cannot hold a lone surrogate, and
no JCS crate operating on `&str`/`String` can ever be handed one.

`serde_json` **errors, it does not replace** — **VERIFIED**:

| Input | `serde_json::from_str` result |
|---|---|
| `{"k":"\ud800"}` | `Err: unexpected end of hex escape at line 1 column 13` |
| `{"k":"\ud800x"}` | `Err: unexpected end of hex escape at line 1 column 13` |
| `{"k":"\ud800\ud800"}` | `Err: lone leading surrogate in hex escape at line 1 column 18` |
| `{"k":"\udc00"}` (low first) | `Err: lone leading surrogate in hex escape at line 1 column 12` |
| `{"k":"\udfff"}` | `Err: lone leading surrogate in hex escape at line 1 column 12` |
| `{"k":"\ud83d\ude00"}` (valid pair) | `Ok` → `{"k":"😀"}` from all crates |
| raw WTF-8 bytes `ED A0 80` via `from_slice` | `Err: invalid unicode code point at line 1 column 10` |

For contrast, `String::from_utf8_lossy(&[0xED,0xA0,0x80])` gives `"\u{FFFD}\u{FFFD}\u{FFFD}"` — but **no JCS crate
calls a lossy conversion on input**. `serde_jcs`, `json-canon` and `serde_json_canonicalizer` do call
`String::from_utf8_unchecked` on their *own output*, which is sound because they only ever emit valid UTF-8.

**Consequence (INFERRED, not tested against a live peer):** a JavaScript or Java JCS implementation *can* be
handed a lone surrogate (JS strings are UTF-16 sequences, not scalar-value sequences) and will canonicalize it
to `\ud800`; every Rust implementation will hard-error at parse. This is a *fail-closed* divergence, not a
silent-corruption one.

### 4.4 Other string-level behaviour — **VERIFIED**, all six crates agree

| Case | All crates |
|---|---|
| `"a/b"` (solidus) | `"a/b"` — **not** escaped ✅ |
| `\/` in input (cyberphone `values.json`) | emitted as `/` ✅ |
| `\u001F` (C0 control) | `\u001f` — **lowercase** hex ✅ |
| `U+007F` (DEL) | emitted literally, not escaped ✅ |
| Unnormalized `A\u030a` | passed through as `Å` (2 code points), no NFC ✅ (RFC 8785 does not normalize) |

Two source-level caveats found by reading, **verified to be unreachable in practice**:
- `json-canon`'s `write_char_escape` maps `CharEscape::Solidus` to `b"\\/"` (i.e. it *would* escape solidus)
  [11]. `serde_json` never emits `CharEscape::Solidus` with the default escape table, so this branch is dead —
  confirmed by the measured `"a/b"` → `"a/b"` result.
- `serde_json_canonicalizer` handles the same case correctly with an explicit comment [16]:
  ```rust
  Solidus => {
      // This is according to the javascript reference implementation (https://www.npmjs.com/package/canonicalize) where
      // an escaped solidus is turned into a non escaped one, in javascript "\/" === "/".
      // RFC 8785 in Section 3.2.2.2 does not list a solidus as a special escape character.
      return self.get_writer(writer).write_all(b"/");
  }
  ```

---

## 5. Interop failure reports, and cyberphone test-suite status (e)

### 5.1 cyberphone/json-canonicalization corpus — I ran it myself against every crate

I extracted the corpus vendored verbatim in `jcs-canonicalize` 0.2.1
(`tests/fixtures/cyberphone-corpus/`, whose `NOTICE.md` states it is "copied verbatim from … 
https://github.com/cyberphone/json-canonicalization, Copyright 2018 Anders Rundgren" [28]) and ran all six
input/output pairs through each crate.

**VERIFIED — byte-identical PASS for all 6 files × all 5 implementations:**

| | arrays | french | structures | unicode | values | weird |
|---|---|---|---|---|---|---|
| `serde_jcs` 0.2.0 | PASS | PASS | PASS | PASS | PASS | PASS |
| `serde_json_canonicalizer` 0.3.2 | PASS | PASS | PASS | PASS | PASS | PASS |
| `json-canon` 0.1.3 | PASS | PASS | PASS | PASS | PASS | PASS |
| `canon-json` 0.2.1 | PASS | PASS | PASS | PASS | PASS | PASS |
| `vr-jcs` 0.4.1 | PASS | PASS | PASS | PASS | PASS | PASS |
| `serde_jcs` **0.1.0** | not run per-file, but **FAILS `weird`** (§3.2, key order) | | | | | |

**Important caveat: the 6-file corpus is a weak filter.** It does not exercise `-0`, NaN, non-BMP key ordering
against `U+E000..U+FFFF`, integers above 2^53, duplicate keys, or `arbitrary_precision`. Every divergence
documented in this report is invisible to it. The corpus also has a much larger companion —
`testdata/es6testfile100m.txt.gz` (100 million IEEE-754 → ES-string pairs) at
https://github.com/cyberphone/json-canonicalization/tree/master/testdata — which I did **NOT** run (**NOT CHECKED**).

Which crates test the corpus in CI:
- `jcs-canonicalize` 0.2.1 — **yes**, vendored, `cyberphone_reference_corpus_byte_identical` + idempotence test [28]
- `serde_json_canonicalizer` 0.3.2 — **yes**, `tests/testdata.rs` walks `tests/resources/testdata/input`, comparing
  both the string and a hex byte-vector (`outhex`) form, with the comment *"test all files from the testdata copied
  from the reference implementation"* (https://docs.rs/crate/serde_json_canonicalizer/0.3.2/source/tests/testdata.rs)
- `json-canon` 0.1.3 — **yes**, `tests/fixtures.rs` `include_str!("../../../test-data/input/arrays.json")` etc.
  (workspace-relative; not shipped in the crate tarball) plus fuzz-derived fixtures
  (https://docs.rs/crate/json-canon/0.1.3/source/tests/fixtures.rs)
- `serde_jcs` 0.2.0 — **partially**; commit *"Add RFC test data"* 2026-03-25 [9] added `tests/basic.rs` covering
  RFC 8785 Appendix B numbers; issue #2 "Test with test data referenced from the specification" closed same day [8]
- `vr-jcs` 0.4.1 — ships `tests/conformance.rs` and `tests/differential.rs`; **I did not read them**, but I ran
  the corpus against it externally: PASS. **NOT CHECKED** whether its own suite includes the corpus.
- `canon-json` 0.2.1 — **no corpus found in the crate tarball**; its dev-dependencies are `cjson`, `olpc-cjson`,
  `proptest`, `sha2`, `similar-asserts`, i.e. property tests + differential comparison against two *other*
  canonical-JSON crates (which implement OLPC Canonical JSON, a different spec).

### 5.2 Reported interop failures — complete list of what I found

**`l1h3r/serde_jcs`**
- **#1 (2020-12-29 → closed 2026-03-25)** "Sorting properties as unescaped UTF-16" [6]. Confirmed real, confirmed
  fixed only in 0.2.0. Reporter: *"I confirmed that the expected output is generated by a different JCS
  implementation, the [json-canonicalize](https://www.npmjs.com/package/json-canonicalize) npm module."*
  Detected via W3C JSON-LD test `toRdf-manifest#tjs13`.
- **#3 (2021-09-09, STILL OPEN)** "Encoding numbers does not follow specification" [7]: *"the official go
  implementation reduces the precision when encoding `uint64`, while serde_jcs does not, leading to a different
  result."* Cross-links cyberphone #20. **Behaviourally fixed in 0.2.0 (verified) but the issue was never closed.**
- #2 (2021-09-07 → closed 2026-03-25) "Test with test data referenced from the specification" [8].
- #4 (PR, merged 2026-03-25) "Updates" — the 0.2.0 rewrite.

**`ahdinosaur/json-canon`**
- **#6 (2023-05-13, closed same day)** "Rust bug: large integers (beyond JavaScript's Number.MAX_SAFE_INTEGER)
  should error" [13]. Body: *"at the moment, we serialize them as is, which could lead to the following problem:
  `JSON.stringify(JSON.parse("[1152921504606846976,…]"))` → `'[1152921504606847000,…]'`"*, and *"another option
  is we cast integers as f64, but this is lossy and i'd rather throw an error than silently convert data."*
  Cross-links `l1h3r/serde_jcs#3` and `cyberphone#20`. Resolved by the error-on-overflow policy that I verified.
- **#5 (2023-05-13, closed same day)** "Rust bug: number keys are not properly handled" [14], with the failing
  assertion pasted: `left: "{\"2\":\"Two\",\"3\":\"Three\",\"\\n\":\"Newline\",\"1\":\"One\",\"4\":\"Four\"}"`
  vs `right: "{\"\\n\":\"Newline\",\"1\":\"One\",\"2\":\"Two\",\"3\":\"Three\",\"4\":\"Four\"}"`. The author's
  note: *"i keep thinking that the Rust is deserialized to `serde_json::Value` (where keys can only be strings)
  before being serialized, which is just not the case."*
- #4, #1 — fuzz testing (#1 opened 2023-05-10; PR #4 "Fuzz tests (and Rust fixes)").
- **Repo last pushed 2023-05-23; issue #11 (object pooling PR) still open.** No commits in 3 years.

**`evik42/serde-json-canonicalizer`**
- **#5 (2025-07-07 → closed 2025-08-10)** "Numbers are output with ff as a prefix" [17]. Exact text:
  *"The test below will output `{"number":ff300}` if serde_json has arbitrary_precision turned on, otherwise it
  outputs the expected json."* — a genuine output-corruption bug triggered purely by a **transitive Cargo feature**.
  Fixed by PR #6 "fix: remove ff from str_number" (2025-07-08), released in 0.3.1 (2025-08-10).
- **#9 (2026-02-06 → closed 2026-02-11)** "Object properties" [18]: *"does this guarantee sorted order for object
  keys? E.g. if the `preserve_order` serde_json feature is enabled?"*
- #10 (2026-02-06, closed) "Questions: Comparison to serde_json::to_string and approach to stability".
- #1 (2023-09-04, closed) "serde_json_canonicalizer vs serde_jcs".
- #4 (2025-01-08, **open**) "no_std support" — not interop-related.

**`bootc-dev/canon-json-rs`** (crates.io still lists the old `containers/canon-json-rs` URL, which 301-redirects)
- **#16 (2026-03-12, STILL OPEN)** "Fails to compile when `arbitrary_precision` and `raw_value` are enabled for
  `serde_json`" [22] — `error[E0275]: overflow evaluating the requirement '&mut Vec<u8>: std::io::Write'` from
  the recursive `WriterTarget<'_, &mut WriterTarget<'_, …>>` nesting. Not an output divergence, but it means
  canon-json is **unusable** in any dependency graph where something enables both features.
- #7 (closed) "Bus factor 1". Remaining open items are Renovate/CI PRs.
- **No issue reports any output divergence.**

**`VertRule/vr-jcs`** — 0 stars, **zero issues, zero PRs** ever filed [23]. Repo pushed 2026-06-12.
**`arcanesys/jcs-canonicalize`** — 0 stars, **zero issues, zero PRs** ever filed [27]. Repo pushed 2026-05-20.

### 5.3 Absence of evidence — explicitly stated

Per the parent's instruction that absence is a wanted result:

- **`serde_json_canonicalizer` 0.3.2**: **no** open or closed issue, PR, or commit message reports an *output*
  divergence from another RFC 8785 implementation. The only output bug ever reported (#5, the `ff` prefix) was a
  feature-unification artifact, not an interop disagreement, and it is fixed. I could not find any report of it
  disagreeing with the JS/Go/Java reference implementations.
- **`canon-json` 0.2.1 / `bootc-dev/canon-json-rs`**: **no** issue or PR reports an output divergence. The
  integer >2^53 pass-through and NaN→`null` behaviours I measured are **undocumented and unreported** — I found
  them by execution, not from any existing report.
- **`vr-jcs` 0.4.1** and **`jcs-canonicalize` 0.2.1**: **no** issues or PRs of any kind exist. Zero community
  scrutiny. `vr-jcs` has 820 lifetime downloads, `jcs-canonicalize` 388.
- **`json-canon` 0.1.3**: **no** *open* interop issue. Both interop bugs (#5, #6) were found and fixed by the
  author in a single day in May 2023. Nothing has been reported since — but nothing has been *changed* since either.
- I found **no** GitHub issue, in any of these repos, reporting a failure against the cyberphone corpus itself.

---

## 6. Two cross-cutting hazards not tied to any single crate

### 6.1 `arbitrary_precision` feature unification silently breaks two crates — **VERIFIED**

Cargo unifies features across a dependency graph. If *any* crate anywhere in your tree enables
`serde_json/arbitrary_precision`, `serde_json` stores every number as its original source string and routes it
through `Formatter::write_number_str`. I built `/tmp/apbtest` with `serde_json = { features = ["arbitrary_precision"] }`:

| JSON input | `serde_jcs` 0.2.0 | `serde_json_canonicalizer` 0.3.2 | **`json-canon` 0.1.3** | **`canon-json` 0.2.1** |
|---|---|---|---|---|
| `1.50` | `1.5` ✅ | `1.5` ✅ | **`1.50`** ❌ | **`1.50`** ❌ |
| `10.0` | `10` ✅ | `10` ✅ | **`10.0`** ❌ | **`10.0`** ❌ |
| `0.000000000000000000000000001` | `1e-27` ✅ | `1e-27` ✅ | **`0.000000000000000000000000001`** ❌ | **same** ❌ |

Root cause, confirmed in source: `json-canon`'s `write_number_str` is
`writer.write_all(value.as_bytes())` [11] and `canon-json`'s is `CompactFormatter.write_number_str(...)` [21] —
both raw pass-through. `serde_jcs` reparses (`self.write_f64(writer, value.parse()...)` [4]) and
`serde_json_canonicalizer` reparses with the comment *"To be JCS conformant the string is parsed into a double
and reformatted"* [16].

Note `0.000000000000000000000000001` → `1e-27` is one of the exact values in the RFC 8785 §3.2.3 worked example [2],
so this is a demonstrable spec violation, not a corner case. It fires **silently, at a distance**, with no
compile error and no runtime error. This is the same class of bug as sjc issue #5 [17], which was reported and fixed.

**Concrete trigger available today:** `vr-jcs` 0.4.1 declares `serde_json` with
`features = ["arbitrary_precision", "preserve_order", "unbounded_depth"]` **non-optionally** [23]. Adding `vr-jcs`
to a workspace that also uses `json-canon` or `canon-json` will silently corrupt the latter's output (or, per
canon-json issue #16 [22], fail to compile if `raw_value` is also on). **INFERRED** — I verified the feature effect
directly by enabling the feature by hand, and read vr-jcs's manifest, but did not build the exact combined graph.

### 6.2 Duplicate object keys — a genuine four-way split — **VERIFIED**

RFC 8785 says nothing normative about duplicate property names (I searched §3.2.3 and the whole RFC text; it
inherits I-JSON, which says names SHOULD be unique). Through the `serde_json::Value` path this is moot —
`serde_json`'s parser deduplicates (last-wins) before canonicalization, and all four produce `{"a":2}` for
`{"a":1,"a":2}`. But via a hand-written `Serialize` impl that calls `serialize_entry` twice with the same key
(`/tmp/jcsdiff/src/bin/dupkeys.rs`, keys `a=1, b=9, a=2`):

| Crate | Output | Semantics |
|---|---|---|
| `serde_json` (baseline) | `{"a":1,"b":9,"a":2}` | keeps both |
| `serde_jcs` 0.2.0 | `{"a":2,"b":9}` | **last wins** (`BTreeMap::insert` overwrites) |
| **`serde_json_canonicalizer` 0.3.2** | **`{"a":1,"b":9}`** | **FIRST wins** (`BTreeSet::insert` is a no-op on an equal element) |
| **`json-canon` 0.1.3** | **`{"a":1,"a":2,"b":9}`** | **keeps both** — emits a duplicate key in "canonical" output |
| `canon-json` 0.2.1 | `{"a":2,"b":9}` | last wins |
| `vr-jcs` 0.4.1 (byte API) | `Err: duplicate property name 'a'` | **rejects** |

Four different answers for the same input. If two services sign the same logical document and one of them
constructs the map via a custom `Serialize` impl, signatures will not verify across this boundary.

### 6.3 vr-jcs rejects inputs the RFC permits — **VERIFIED**

`vr-jcs` rejects Unicode noncharacters in strings and property names — `validate_string_contents` errors on
`U+FDD0..=U+FDEF` and any `code & 0xFFFE == 0xFFFE` [26]:

```
{"k":"\u{FFFF}"}  vr-jcs → ERR(string value contains the forbidden noncharacter U+FFFF)
{"k":"\u{FDD0}"}  vr-jcs → ERR(string value contains the forbidden noncharacter U+FDD0)
```
All four other crates emit these code points unchanged (**VERIFIED**). RFC 8785 contains no such prohibition;
this is an I-JSON/RFC 7493 hardening choice layered on top. It is fail-closed, but it means vr-jcs will refuse
to canonicalize documents that every other implementation accepts. Its README advertises this as
"I-JSON string / number validation", so it is deliberate and documented [23].

---

## Recommendation summary (for the parent — clearly an opinion, not a source claim)

1. **`serde_json_canonicalizer` 0.3.2** is the best-maintained choice: most downloads, most recent bugfix
   cadence, ryu-js 1.x, UTF-16 sorting, errors on NaN/Infinity at the Serializer layer, immune to
   `arbitrary_precision`, and has a real reference-testdata suite committed. Its one quirk is **first-wins**
   duplicate-key resolution, which differs from every other crate.
2. **`serde_jcs` — require `>= 0.2.0` explicitly.** 0.1.0 is unfixably wrong on key order and is still receiving
   ~570k downloads per 90 days. If you depend on it transitively, audit `Cargo.lock`.
3. **Avoid `json-canon` and `canon-json` if any crate in your graph might enable `serde_json/arbitrary_precision`,**
   and avoid both if NaN can reach the serializer from a native Rust value.
4. **`canon-json`'s integer pass-through** (`9007199254740993` stays exact) makes it incompatible with the JS/Go/Java
   reference implementations for large integers. That is a signature-breaking divergence, not a rounding nicety.

---

## Coverage Status

**Checked directly (source read + code executed):**
- `serde_jcs` 0.1.0 and 0.2.0, `serde_json_canonicalizer` 0.3.2, `json-canon` 0.1.3, `canon-json` 0.2.1,
  `vr-jcs` 0.4.1, `jcs-canonicalize` 0.2.1 — full library source read from the published `.crate` tarballs.
- Float backend identity + measured ryu / ryu-js / Rust-Display divergence on all six values the parent named.
- Key sorting via the discriminating non-BMP pair `U+20BB7` vs `U+FFA5` (and `U+10000` vs `U+E000`).
- `-0.0`, NaN, Infinity, lone/unpaired surrogates, raw WTF-8, solidus, DEL, C0 controls, noncharacters.
- Full cyberphone 6-file corpus against all 5 implementations, byte-for-byte.
- Duplicate-key behaviour through the non-`Value` `serialize_map` path.
- `arbitrary_precision` feature-unification corruption.
- All GitHub issues + PRs (open and closed) for all six repos, plus cyberphone issue #20; issue bodies quoted verbatim.
- RFC 8785 §3.2.2.3, §3.2.3, §3.2.4 quoted from the RFC Editor text.

**Uncertain / marked INFERRED in the text:**
- That adding `vr-jcs` to a graph containing `json-canon`/`canon-json` corrupts the latter — I verified the
  feature *effect* by enabling `arbitrary_precision` manually and read vr-jcs's manifest, but did not build the
  exact combined dependency graph.
- Cross-language lone-surrogate behaviour of JS/Java JCS implementations (reasoned from UTF-16 string semantics,
  not tested against a live peer).

**Not completed:**
- `testdata/es6testfile100m.txt.gz` (100 M ECMAScript number test vectors) was **not** run against any crate.
  This is the strongest available float-formatting check and the only remaining gap in the number analysis.
- Source of the seven ecosystem-specific crates in §1.2 (`acdp-jcs`, `boundary-compiler`, `reallyme-codec-jcs`,
  `canaad-core`, `warpin-integrity`, `assay-canonical`, `vauban-x402-jcs-conformance`) was **not** read; only
  crates.io metadata and declared dependencies were checked. `acdp-jcs` in particular declares **no** ryu-js
  dependency and its float path is unknown.
- `vr-jcs`'s own `tests/conformance.rs` and `tests/differential.rs` were **not** read (I tested it externally instead).
- I did not attempt to find non-GitHub interop reports (mailing lists, forums, Discord).

---

## Sources

1. RFC 8785, "JSON Canonicalization Scheme (JCS)", §3.2.2.3 — https://www.rfc-editor.org/rfc/rfc8785.txt
2. RFC 8785 §3.2.3 "Sorting of Object Properties" — https://www.rfc-editor.org/rfc/rfc8785.txt
3. crates.io — serde_jcs — https://crates.io/crates/serde_jcs
4. serde_jcs 0.2.0 `src/lib.rs` — https://docs.rs/crate/serde_jcs/0.2.0/source/src/lib.rs
5. serde_jcs 0.1.0 `src/entry.rs` — https://docs.rs/crate/serde_jcs/0.1.0/source/src/entry.rs
6. l1h3r/serde_jcs issue #1, "Sorting properties as unescaped UTF-16" — https://github.com/l1h3r/serde_jcs/issues/1
7. l1h3r/serde_jcs issue #3, "Encoding numbers does not follow specification" — https://github.com/l1h3r/serde_jcs/issues/3
8. l1h3r/serde_jcs issue #2, "Test with test data referenced from the specification" — https://github.com/l1h3r/serde_jcs/issues/2
9. l1h3r/serde_jcs commit history — https://github.com/l1h3r/serde_jcs/commits/main
10. crates.io — json-canon — https://crates.io/crates/json-canon
11. json-canon 0.1.3 `src/ser.rs` — https://docs.rs/crate/json-canon/0.1.3/source/src/ser.rs
12. json-canon 0.1.3 `src/object.rs` — https://docs.rs/crate/json-canon/0.1.3/source/src/object.rs
13. ahdinosaur/json-canon issue #6 — https://github.com/ahdinosaur/json-canon/issues/6
14. ahdinosaur/json-canon issue #5 — https://github.com/ahdinosaur/json-canon/issues/5
15. crates.io — serde_json_canonicalizer — https://crates.io/crates/serde_json_canonicalizer
16. serde_json_canonicalizer 0.3.2 `src/jcs.rs` — https://docs.rs/crate/serde_json_canonicalizer/0.3.2/source/src/jcs.rs
17. evik42/serde-json-canonicalizer issue #5 — https://github.com/evik42/serde-json-canonicalizer/issues/5
18. evik42/serde-json-canonicalizer issue #9 — https://github.com/evik42/serde-json-canonicalizer/issues/9
19. crates.io — canon-json — https://crates.io/crates/canon-json
20. canon-json 0.2.1 `src/floatformat.rs` — https://docs.rs/crate/canon-json/0.2.1/source/src/floatformat.rs
21. canon-json 0.2.1 `src/lib.rs` — https://docs.rs/crate/canon-json/0.2.1/source/src/lib.rs
22. bootc-dev/canon-json-rs issue #16 — https://github.com/bootc-dev/canon-json-rs/issues/16
23. crates.io — vr-jcs — https://crates.io/crates/vr-jcs
24. vr-jcs 0.4.1 `src/number.rs` — https://docs.rs/crate/vr-jcs/0.4.1/source/src/number.rs
25. vr-jcs 0.4.1 `src/canonicalize.rs` — https://docs.rs/crate/vr-jcs/0.4.1/source/src/canonicalize.rs
26. vr-jcs 0.4.1 `src/strict_parse.rs` — https://docs.rs/crate/vr-jcs/0.4.1/source/src/strict_parse.rs
27. crates.io — jcs-canonicalize — https://crates.io/crates/jcs-canonicalize
28. jcs-canonicalize 0.2.1 `tests/cyberphone_corpus.rs` — https://docs.rs/crate/jcs-canonicalize/0.2.1/source/tests/cyberphone_corpus.rs
29. cyberphone/json-canonicalization (RFC 8785 reference impl + testdata) — https://github.com/cyberphone/json-canonicalization
30. cyberphone/json-canonicalization issue #20, "Support for uint64" — https://github.com/cyberphone/json-canonicalization/issues/20
31. boa-dev/ryu-js README — https://github.com/boa-dev/ryu-js
32. crates.io — ryu-js — https://crates.io/crates/ryu-js
33. crates.io — zmij — https://crates.io/crates/zmij
34. crates.io download API — serde_jcs per-version — https://crates.io/api/v1/crates/serde_jcs/downloads
35. evik42/serde-json-canonicalizer README — https://github.com/evik42/serde-json-canonicalizer#readme
36. serde_jcs 0.2.0 `tests/basic.rs` (RFC Appendix B vectors) — https://docs.rs/crate/serde_jcs/0.2.0/source/tests/basic.rs
37. serde_json_canonicalizer 0.3.2 `tests/testdata.rs` — https://docs.rs/crate/serde_json_canonicalizer/0.3.2/source/tests/testdata.rs
38. json-canon 0.1.3 `tests/fixtures.rs` — https://docs.rs/crate/json-canon/0.1.3/source/tests/fixtures.rs
