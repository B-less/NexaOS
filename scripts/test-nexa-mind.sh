#!/bin/sh
# End-to-end test of Nexa Mind (milestone M1) in a clean Alpine container:
# build nexa-mindd, install the default model from the local cache with the
# same size + SHA-256 check a download goes through, start the service, ask
# it a question, then run the assistant's agent loop on the local back-end
# and check that the on-device model answers by calling a real tool.
#
# Needs the model in /var/tmp/nexa-models (see docs/NEXA_MIND.md).
# Usage: sh scripts/test-nexa-mind.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
model=/var/tmp/nexa-models/qwen2.5-1.5b-instruct-q4_k_m.gguf
[ -s "$model" ] || { echo "missing $model"; exit 1; }
sh "$repo/scripts/build-native.sh" >/dev/null   # cached: only changes recompile
image=nexa-mindrun:3.24
# Built once and reused: just what the service needs at run time.
if ! docker image inspect "$image" >/dev/null 2>&1; then
    printf 'FROM alpine:3.24\nRUN apk add -q llama-server llama.cpp-cpu curl python3\n' \
        | docker build -q -t "$image" - >/dev/null
fi
docker run --rm -v "$repo:/src:ro" -v /var/tmp/nexa-models:/models:ro \
    "$image" sh -c '
set -e
M=/src/build/native/nexa-mindd
export NEXA_MIND_STATE=/var/lib/nexa-mind NEXA_MIND_RUN=/run/nexa-mind NEXA_MIND_SOCKET=/run/nexa-mind/mind.sock
mkdir -p $NEXA_MIND_STATE/models
# Install exactly as a finished download would be: size and SHA-256 must match the catalogue.
want=6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e
got=$(sha256sum /models/qwen2.5-1.5b-instruct-q4_k_m.gguf | cut -d" " -f1)
[ "$got" = "$want" ] || { echo "FAIL: cached model has the wrong checksum"; exit 1; }
# Copied in, not linked: the Landlock sandbox only lets the model server
# read inside the state folder of the service.
cp /models/qwen2.5-1.5b-instruct-q4_k_m.gguf $NEXA_MIND_STATE/models/
echo "$want" > $NEXA_MIND_STATE/models/qwen2.5-1.5b-instruct-q4_k_m.gguf.verified

$M serve > /tmp/mindd.log 2>&1 &
for i in $(seq 50); do [ -S $NEXA_MIND_SOCKET ] && break; sleep 0.2; done
echo "--- service"; head -2 /tmp/mindd.log
$M status | python3 -c "import json,sys; s=json.load(sys.stdin); print(\"status: state=%s threads=%s/%s installed=%s landlock_abi=%s\" % (s[\"state\"], s[\"threads\"], s[\"threads_batch\"], s[\"installed\"], s[\"landlock_abi\"]))"

echo "--- plain question (model loads on first use)"
start=$(date +%s)
answer=$($M ask "What is the capital of Ghana? Answer with just the city name.")
echo "answer: $answer   ($(( $(date +%s) - start )) s including loading the model)"
echo "$answer" | grep -qi accra || { echo "FAIL: unexpected answer"; tail -20 $NEXA_MIND_STATE/llama-server.log; exit 1; }
$M status | grep -q "\"ready\"" && echo "ok  model loaded and ready"

echo "--- the assistant on the local back-end, with tools"
cd /src/nexa-ai && PYTHONPATH=/src/nexa-ai:/src/common python3 - <<PY
import time
from nexa_ai import agent
events = []
t = time.time()
agent.run(None, None, "How much RAM does this computer have in total? Check with a tool.", False, True,
          lambda *a: True, lambda *a, **k: None, lambda kind, p: events.append((kind, p)), backend="local")
calls = [p["name"] for k, p in events if k == "tool_call"]
texts = [p for k, p in events if k == "text"]
print("tool calls:", calls)
print("answer:", " ".join(texts)[:300])
print("took %.0f s" % (time.time() - t))
assert "get_mem_info" in calls, "the model did not use the memory tool"
assert texts, "no answer text"
print("ok  the on-device model called a real tool and answered")
PY
echo "--- unload"
$M status >/dev/null && echo "{}" > /dev/null
python3 -c "
import socket,json
s=socket.socket(socket.AF_UNIX); s.connect(\"/run/nexa-mind/mind.sock\")
s.sendall(b\"POST /v1/unload HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n\"); print(s.recv(4096).split(b\"\r\n\")[0].decode())"
$M status | grep -q "\"idle\"" && echo "ok  model unloaded on request"
echo PASS
'
