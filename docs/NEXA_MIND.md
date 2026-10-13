# Nexa Mind — the on-device AI, from the hardware up

Written 2026-10-13. Nexa Mind is the part of NexaOS that understands and
acts: a language model that runs on the laptop itself, a private index of
your files, a permission system for anything it does, and the places you
meet it (Nexus search, the assistant window, voice, "ask about this
screen"). Claude stays as an optional online mode; nothing about your
files leaves the machine unless you send it.

This document goes layer by layer, bottom up. Numbers in "Measured on the reference machine" were measured there;
the rest are targets.

**Reference machine:** HP EliteBook 840 G6 — Intel Core i5-8365U (4 cores /
8 threads, AVX2 + FMA, no AVX-512, no NPU), Intel UHD 620, 8 GB RAM.
NexaOS has to be good on this, not on a workstation.

---

## L0 — Kernel and hardware

What the model needs from the machine, and what the kernel must provide
so the AI never makes the laptop feel slow.

| Need | How | Kernel (6.18, monolithic) |
|---|---|---|
| Fast matrix maths | ggml's CPU back-end picks the best code path at runtime (AVX2/FMA here) | — |
| Weights without copying | GGUF files are **mmap'ed**: pages come from the page cache, shared between processes, dropped under memory pressure and reloaded from disk | — |
| Keep the AI in its lane | the model runs in its own **cgroup v2**: `cpu.weight` low (your apps win), `memory.high` caps it, `cpuset` can keep a core free | MEMCG, CGROUP_SCHED, CPUSETS: on |
| Back off when you're busy | read **PSI** (`/proc/pressure/{cpu,memory,io}`); background jobs (indexing) pause when pressure rises | **PSI: off → enable** |
| Don't heat the laptop for background work | **utilisation clamping** (`cpu.uclamp.max`) on background jobs keeps clocks low; interactive answers run unclamped | **UCLAMP_TASK: off → enable** |
| Know the battery cost | **RAPL** energy counters (package watts) for the power governor and for Pulse | **POWERCAP + INTEL_RAPL: off → enable** |
| Faster big allocations | **transparent huge pages** (`madvise` mode) for the KV cache and buffers | **TRANSPARENT_HUGEPAGE: off → enable (madvise)** |
| Watch files to index | **inotify** per user (fanotify isn't needed for a home folder) | INOTIFY_USER: on |
| Sandbox tools | **Landlock** + **seccomp** for the tool runner; bubblewrap for apps | LANDLOCK, SECCOMP_FILTER: on |
| Memory headroom | zram swap (already on) so a model load never freezes the desktop | ZRAM: on |
| GPU (optional) | Vulkan back-end on the UHD 620 (`libggml-vulkan`); only used if the benchmark shows it beating the CPU | DRM_I915: on |

## L1 — Inference runtime

**llama.cpp**, as packaged by Alpine (`llama.cpp`, `llama-server`,
`libggml-cpu`, `libggml-vulkan`, `whisper.cpp`). No Python ML stack, no
GPU drivers to ship: one C++ server, ~14 MB.

- `llama-server` runs as the system service **`nexa-mind`**, as its own
  user, in its own cgroup, listening on a **Unix socket**
  (`/run/nexa-mind/llm.sock`, group `nexa-mind`, mode 0660) — never a TCP
  port, so nothing on the network can reach it.
- OpenAI-compatible API with **tool calling** (`--jinja`), **streaming**,
  and **embeddings** (a second, tiny embedding model on its own socket).
- Settings chosen by a one-time **auto-tune** (`llama-bench` on first run,
  stored in `/var/lib/nexa-mind/tune.json`): threads = physical cores for
  generation, all threads for prompt processing, flash attention on, KV
  cache quantised to q8_0, context 8 k.
- **Prompt caching:** the system prompt + tool list are evaluated once and
  their KV state saved (`--slot-save-path`); later questions start from it,
  so the first word appears fast.
- **Lazy load, idle unload:** the model loads on the first question and is
  unloaded after 10 idle minutes (memory back to you); because weights are
  mmap'ed from disk, reloading mostly re-reads the page cache.

### Models

Never on the ISO (size, licences): downloaded in Settings > Nexa Mind, over
HTTPS, each pinned by **SHA-256** in a manifest shipped with NexaOS and
checked before first use. Only permissively licensed models.

| Role | Model (GGUF) | Size | Licence | Fits |
|---|---|---|---|---|
| Chat + tools, default | Qwen2.5-1.5B-Instruct Q4_K_M | 1.1 GB | Apache-2.0 | 8 GB laptops |
| Chat + tools, larger | Qwen2.5-7B-Instruct Q4_K_M / Phi-3.5-mini (MIT) | 4.7 / 2.4 GB | Apache-2.0 / MIT | 16 GB+ |
| Embeddings (search) | nomic-embed-text-v1.5 Q8_0 | 0.14 GB | Apache-2.0 | all |
| Speech to text | whisper base.en | 0.14 GB | MIT | all |
| Voice out | espeak-ng (no model) | — | GPL-3 | all |

### Measured on the reference machine

`llama-bench`, Alpine's llama.cpp (AVX2 code path picked automatically),
Qwen2.5-1.5B-Instruct Q4_K_M (1.04 GiB, SHA-256 `6a1a2eb6…e9407e`), under
WSL2 on the HP itself, 2026-10-13:

| Threads | Prompt reading | Answer writing |
|---|---|---|
| **4** (one per core) | **40 tokens/s** | **12.3 tokens/s** |
| 8 (with hyper-threads) | 36 tokens/s | 9.5 tokens/s |

What follows from it:
- **Writing at ~12 tokens/s (~9 words/s) is faster than people read**: the
  1.5B model is the right default for 8 GB machines.
- **Threads = physical cores.** Hyper-threads made it 23 % slower; the
  auto-tune must measure, not assume.
- **Prompt reading is the bottleneck**: a 1,000-token system prompt plus
  tool list would take ~25 s before the first word. Prompt caching
  (`--slot-save-path`, cache reuse) is therefore not optional: the fixed
  part is evaluated once and reloaded, and the tool list is kept short and
  loaded on demand per topic.
- 7B-class models (~4× slower) are for 16 GB+ machines with patience or a
  real GPU.

## L2 — The Nexa Mind service

`nexa-mindd`, one per user session, is the only thing apps talk to. It
owns: the request queue, the user's context index, tool execution, and
the connection to the model server (local) or Claude (online, optional).

- **API:** D-Bus session service `os.nexa.Mind1` — `Ask(text, context) →
  request id`, streamed `Token`/`ToolCall`/`Done` signals, `Cancel`,
  `Search(query)`, `Status()`. Apps never see the model socket.
- **Priorities:** interactive questions pre-empt background work; one
  model instance, a queue in front of it; `Cancel` stops generation
  mid-sentence (Escape in the UI).
- **Resource governor:** background jobs (indexing, summaries) only run
  when PSI is low, on mains power or above 50 % battery, and with
  `uclamp.max` set; interactive answers get full speed. On battery the
  governor picks fewer threads.
- **Routing:** local first. "Ask Claude instead" is an explicit button per
  answer, shows exactly what will be sent, and never includes indexed file
  content unless you tick it.

## L3 — Tools and the permission broker

The assistant acts through the same tool layer NexaOS already has
(`nexa_ai/tools.py`), upgraded from "read vs control" to **capabilities**:

| Capability | Examples | Default |
|---|---|---|
| `system.read` | CPU, memory, disks, logs | allow |
| `files.read:<folder>` | read/search documents | allow inside home, never `~/.ssh`, keyrings, browser profiles |
| `files.write:<folder>` | create, move, rename | ask |
| `files.delete` | delete (to a Nexa trash, recoverable) | ask, always |
| `apps.launch`, `notify` | open apps, notifications | allow |
| `settings.write` | Wi-Fi, Bluetooth, theme, VPN | ask |
| `system.control` | services, power, packages | ask + password on installed systems |
| `net.fetch` | read a web page | ask |

- "Ask" shows the **exact action** ("Move 12 files from Downloads to
  Pictures/2026") with Allow once / Always for this / Deny.
- Every action goes to an **audit log** (Settings > Nexa Mind > Activity).
- **Prompt-injection guard:** text from files, web pages and OCR is marked
  *untrusted*; while untrusted text is in the conversation, every action
  needs a fresh confirmation, whatever the defaults say.
- Tools run in a **sandboxed runner** (Landlock limited to the folders the
  capability names, seccomp, no network unless `net.fetch`); privileged
  steps still go through the existing doas helpers.

## L4 — Context engine (your private index)

- **What:** text, Markdown and code files, PDFs (poppler), Office files
  (text extracted from the XML), images (OCR via tesseract), optionally
  voice notes (whisper). Skips hidden folders, `~/.ssh`, password stores,
  browser profiles, anything you exclude in Settings.
- **How:** inotify on your folders + a periodic re-scan; changed files are
  split into ~500-token chunks, embedded by the embedding model, stored in
  **SQLite** (`~/.local/share/nexa-mind/index.db`, mode 0600): **FTS5** for
  words, float16 vectors for meaning; results merged with reciprocal-rank
  fusion. Brute-force vector search is fast enough for a home folder
  (tens of thousands of chunks); an ANN index only if it isn't.
- **Uses:** Nexus search finds files by meaning ("the invoice from the
  phone shop"), the assistant answers from your documents with citations
  (file + page), and nothing ever leaves the machine.
- **Control:** pause indexing, exclude folders, wipe the index — in Settings.

## L5 — Where you meet it

| Surface | What |
|---|---|
| **Nexus** | the search box: apps, settings, sums, and "Ask Nexa" (done) — answers stream in place, file results from the index |
| **Assistant window** | conversations, tool actions with approvals, model switch (local / Claude) |
| **Super+Shift+Space** | quick-ask bubble over any app |
| **Screen** | "Copy text from screen" and "Ask about this screen": a region screenshot (grim) → OCR → into the question |
| **Voice** | hold a key: whisper transcribes, Nexa answers, espeak-ng speaks (all offline) |
| **Files / PDF / Mail** | "Summarise", "Find similar", "Explain" in the apps' menus |

---

## Milestones

| | Milestone | Done when |
|---|---|---|
| M1 | **Runtime**: llama.cpp in the image, model manager with SHA-256 check, `nexa-mind` service on a Unix socket, auto-tune; the assistant's agent talks to it (tool calls included) | offline question answered with a tool call, in QEMU and on the HP |
| M2 | **Kernel**: PSI, UCLAMP_TASK, POWERCAP/RAPL, THP (madvise); cgroup for the model | readings visible in Pulse; model throttled under load |
| M3 | **Permission broker** + audit log + untrusted-content guard | tests: injected file text can't trigger a silent action |
| M4 | **Context index** (text, PDF) + Nexus file search by meaning | finds a file by describing it |
| M5 | **Screen**: copy text from screen, ask about screen | OCR round trip test |
| M6 | **Voice**: push-to-talk in and out | spoken question answered offline |
