# Fixed llama.cpp CPU runtime packaging

Review `llama-cpu-runtime-v1.manifest` before installation. `BUILD_BIN` is the
already verified build output directory; it is intentionally not a persistent
runtime path. These commands copy dereferenced regular files into fixed
root-owned locations and do not install symlinks:

```sh
BUILD_BIN=/path/to/verified/llama.cpp-build/bin
MODEL_FILE=/path/to/verified/nomic-embed-text-v1.5.Q8_0.gguf

sudo install -d -o root -g root -m 0755 /usr/local/lib/neural-memory/llama-cpu
sudo install -o root -g root -m 0644 "$BUILD_BIN/libggml-base.so.0" /usr/local/lib/neural-memory/llama-cpu/libggml-base.so.0
sudo install -o root -g root -m 0644 "$BUILD_BIN/libggml-cpu.so.0" /usr/local/lib/neural-memory/llama-cpu/libggml-cpu.so.0
sudo install -o root -g root -m 0644 "$BUILD_BIN/libggml.so.0" /usr/local/lib/neural-memory/llama-cpu/libggml.so.0
sudo install -o root -g root -m 0644 "$BUILD_BIN/libllama-common.so.0" /usr/local/lib/neural-memory/llama-cpu/libllama-common.so.0
sudo install -o root -g root -m 0644 "$BUILD_BIN/libllama-server-impl.so" /usr/local/lib/neural-memory/llama-cpu/libllama-server-impl.so
sudo install -o root -g root -m 0644 "$BUILD_BIN/libllama.so.0" /usr/local/lib/neural-memory/llama-cpu/libllama.so.0
sudo install -o root -g root -m 0644 "$BUILD_BIN/libmtmd.so.0" /usr/local/lib/neural-memory/llama-cpu/libmtmd.so.0
sudo install -o root -g root -m 0755 "$BUILD_BIN/llama-server" /usr/local/bin/llama-server
sudo install -D -o root -g root -m 0444 deploy/embedding/llama-cpu-runtime-v1.manifest /usr/local/share/neural-memory/llama-cpu-runtime-v1.manifest
sudo install -D -o root -g root -m 0444 deploy/embedding/embedding-provenance-v1.json /usr/local/share/neural-memory/embedding-provenance-v1.json
sudo install -D -o root -g root -m 0755 scripts/neural-memory-verify-llama-runtime-v1.sh /usr/local/libexec/neural-memory-verify-llama-runtime-v1
sudo install -D -o root -g root -m 0755 scripts/neural-memory-wait-embedding-ready-v1.sh /usr/local/libexec/neural-memory-wait-embedding-ready-v1
sudo install -d -o root -g neural-memory -m 0750 /usr/local/share/neural-memory/models
sudo install -o root -g neural-memory -m 0640 "$MODEL_FILE" /usr/local/share/neural-memory/models/nomic-embed-text-v1.5.Q8_0.gguf
```

Run `/usr/local/libexec/neural-memory-verify-llama-runtime-v1` and
`/usr/local/libexec/neural-memory-verify-embedding-model-v1` before installing
or starting the service unit. The runtime verifier rejects symlinks, missing or
extra directory entries, metadata drift, byte-size drift, hash drift, and a
server version mismatch. The service retains `ProtectHome=true` and supplies
only `LD_LIBRARY_PATH=/usr/local/lib/neural-memory/llama-cpu`.

The fixed service uses `--ctx-size 2048 --batch-size 2048 --ubatch-size 2048
--parallel 1`.
This exact sealed GGUF declares `n_ctx_train=2048`; it is authoritative for the
deployed artifact. The upstream model card's current 8192-token claim does not
establish this GGUF's provenance or capability. Equal logical and physical
batches avoid llama.cpp reducing both to 512 for embedding mode, and explicit
serial operation gives each request the full 2048-token slot.

`embedding-provenance-v1.json` is the machine-verifiable provenance convention.
It records the source repository and full commit, build number, reported
version, toolchain, material build flags, model upstream URL and revision (or
the literal `unknown`), declared artifact context, and hashes/sizes connecting
the server, model, and runtime-library manifest. The runtime verifier requires
that root-owned file at the fixed path and seals its literal bytes by SHA-256;
documentation alone is not accepted as provenance.

Install the reviewed unit after the fixed artifacts above:

```sh
sudo install -D -o root -g root -m 0644 deploy/systemd/neural-memory-embedding-server.service /etc/systemd/system/neural-memory-embedding-server.service
sudo systemctl daemon-reload
sudo systemctl restart neural-memory-embedding-server.service
sudo systemctl status neural-memory-embedding-server.service
```

The fixed no-argument `ExecStartPost` helper waits at most 90 seconds and probes
only `http://127.0.0.1:8082/v1/embeddings` with an actual embedding request of
1800 repeated words. The live tokenizer measured this at approximately 1803
tokens: within this artifact's 2048 context but above the former 512 batch.
Connection failures, HTTP 503 responses, and all other non-success responses
remain failures until the deadline. Consequently the service does not reach
the active state, and dependent workers cannot start, merely because the TCP
listener exists before the model is loaded.

The worker treats either observed response—input "too large" or input "larger
than the max context size"—as an over-limit input and a
terminal record-local `input-too-large` failure. It removes that record from
the pending queue, records it as stale without storing text in the error, and
continues with later records. Connectivity and other model failures still abort
the bounded run instead of silently discarding retryable work.
