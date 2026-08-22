#!/usr/bin/env bash
set -euo pipefail
set -f
umask 077

[[ $# -eq 0 ]] || { echo "llama runtime verifier accepts no arguments" >&2; exit 2; }
runtime_dir=/usr/local/lib/neural-memory/llama-cpu
manifest=/usr/local/share/neural-memory/llama-cpu-runtime-v1.manifest
provenance=/usr/local/share/neural-memory/embedding-provenance-v1.json
server=/usr/local/bin/llama-server
expected_manifest_sha256=5bfd7debb88b50edc1748e8284fb2bf676ffc7dc18b2dd27582059566f98c7ac
expected_provenance_sha256=f68c00856e107909fc593c8b448a4e4f26118d068744cceddc07fc2e0e5a9366
expected_server_sha256=bd95aacd01abd53a2eed1a08c3e37f808d1760e7161ca7f5520a452ff81c0fe2
expected_server_bytes=17904
expected_short_commit=d0bfb1981
expected_version=10188

[[ -f $provenance && ! -L $provenance ]] || { echo "fixed embedding provenance manifest unavailable" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%U:%G:%a' -- "$provenance") == root:root:444 ]] || { echo "embedding provenance manifest metadata is unsafe" >&2; exit 1; }
provenance_sha256=$(/usr/bin/sha256sum -- "$provenance")
[[ ${provenance_sha256%% *} == "$expected_provenance_sha256" ]] || { echo "embedding provenance manifest SHA-256 mismatch" >&2; exit 1; }

[[ -f $manifest && ! -L $manifest ]] || { echo "fixed runtime manifest unavailable" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%U:%G:%a' -- "$manifest") == root:root:444 ]] || { echo "runtime manifest metadata is unsafe" >&2; exit 1; }
manifest_sha256=$(/usr/bin/sha256sum -- "$manifest")
[[ ${manifest_sha256%% *} == "$expected_manifest_sha256" ]] || { echo "runtime manifest SHA-256 mismatch" >&2; exit 1; }
[[ -d $runtime_dir && ! -L $runtime_dir ]] || { echo "fixed llama runtime directory unavailable" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%U:%G:%a' -- "$runtime_dir") == root:root:755 ]] || { echo "llama runtime directory metadata is unsafe" >&2; exit 1; }

count=0
while read -r filename expected_bytes expected_sha256 extra; do
    [[ -z ${extra-} && $filename =~ ^lib(llama|ggml|mtmd)[A-Za-z0-9._-]*\.so(\.[0-9]+)?$ && $expected_bytes =~ ^[0-9]+$ && $expected_sha256 =~ ^[0-9a-f]{64}$ ]] || { echo "invalid llama runtime manifest" >&2; exit 1; }
    library=$runtime_dir/$filename
    [[ -f $library && ! -L $library ]] || { echo "llama runtime library unavailable: $filename" >&2; exit 1; }
    [[ $(/usr/bin/stat -c '%s' -- "$library") == "$expected_bytes" ]] || { echo "llama runtime library byte length mismatch: $filename" >&2; exit 1; }
    actual_sha256=$(/usr/bin/sha256sum -- "$library")
    [[ ${actual_sha256%% *} == "$expected_sha256" ]] || { echo "llama runtime library SHA-256 mismatch: $filename" >&2; exit 1; }
    [[ $(/usr/bin/stat -c '%U:%G:%a' -- "$library") == root:root:644 ]] || { echo "llama runtime library metadata is unsafe: $filename" >&2; exit 1; }
    ((count += 1))
done <"$manifest"
[[ $count -eq 7 ]] || { echo "llama runtime manifest must contain seven libraries" >&2; exit 1; }
[[ $(/usr/bin/find "$runtime_dir" -mindepth 1 -maxdepth 1 -printf . | /usr/bin/wc -c) -eq 7 ]] || { echo "unexpected llama runtime directory entry" >&2; exit 1; }

[[ -f $server && ! -L $server && -x $server ]] || { echo "fixed llama-server unavailable" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%s' -- "$server") == "$expected_server_bytes" ]] || { echo "llama-server byte length mismatch" >&2; exit 1; }
actual_server_sha256=$(/usr/bin/sha256sum -- "$server")
[[ ${actual_server_sha256%% *} == "$expected_server_sha256" ]] || { echo "llama-server SHA-256 mismatch" >&2; exit 1; }
[[ $(/usr/bin/stat -c '%U:%G:%a' -- "$server") == root:root:755 ]] || { echo "llama-server metadata is unsafe" >&2; exit 1; }
version_output=$(/usr/bin/env -i LD_LIBRARY_PATH="$runtime_dir" "$server" --version 2>&1)
/usr/bin/grep -Fq "$expected_short_commit" <<<"$version_output" || { echo "llama-server short commit mismatch" >&2; exit 1; }
/usr/bin/grep -Eq "(^|[^0-9])${expected_version}([^0-9]|$)" <<<"$version_output" || { echo "llama-server version mismatch" >&2; exit 1; }
