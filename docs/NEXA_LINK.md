# Nexa Link: your PC and your phone, connected (design, 2026-10-25)

NexaOS on the PC and Nexa Mobile (docs/MOBILE_PLAN.md) find each other and
connect **automatically** - but only after you have paired them once, only
with each other, and only for what you allowed. Nothing goes through a
server or the internet.

## 1. What it does (each part switched on per device, by you)

| Feature | Default |
|---|---|
| Phone notifications on the PC (reply to messages from the PC) | on after pairing |
| Calls: see who's calling, answer or decline on the PC; send texts | on after pairing |
| Battery and signal of the phone in the top bar | on |
| Send files and links either way (the receiver accepts; saved to Downloads) | on, asks each time |
| Find my phone (rings it, even on silent) | on |
| Media controls (pause the phone's music from the PC) | on |
| Shared clipboard | **off** - and never anything marked as a password |
| Use the phone as a remote / touchpad, or the PC's keyboard on the phone | **off**; asks every session |
| Continue where you were (open the page or file the other device has open) | on |

## 2. Is "automatic" safe? Yes, if these hold

Automatic means: two devices **you already paired** connect by themselves
when they are near (same trusted network, or Bluetooth). A device you never
paired never gets anything. What made the best-known Linux-phone link, KDE
Connect, unsafe at times - and the rule here that rules each one out:

| Lesson (KDE Connect) | Nexa Link rule |
|---|---|
| An unpaired device could connect: the identity sent before encryption wasn't checked against the one inside it ([CVE-2025-66270](https://www.sentinelone.com/vulnerability-database/cve-2025-66270/)) | A device **is** its public key (Ed25519). There is no separate "device ID" to trust: the encrypted handshake proves the key, and only keys pinned at pairing are accepted. Unknown keys are dropped before a single message is read |
| The pairing code was short and fixed, so an attacker could make keys until the code matched ([CVE-2025-32898](https://kde.org/info/security/advisory-20250418-3.txt)) | Pairing by **QR code** (the PC shows its key; the phone scans it) or a code that mixes **fresh random values from both devices** (it changes every attempt), confirmed on both screens within 60 s |
| Unauthenticated discovery packets could fake names or unpair devices ([CVE-2025-32899](https://nvd.nist.gov/vuln/detail/cve-2025-32899), [CVE-2025-32900](https://kde.org/info/security/advisory-20250418-2.txt)) | Discovery messages **change nothing** - they only say "a paired device may be here", as a rotating code only your paired devices can recognise (strangers on the network can't even tell it's you). Names, unpairing and everything else happen only inside the encrypted channel |
| Certificate errors were ignored for unknown peers ([CVE-2020-26164](https://seclists.org/oss-sec/2020/q4/56)) | No "accept anyway" path exists in the code |

And beyond those:

- **Encrypted end to end**: Noise protocol (XX pattern, as in WireGuard's
  family) with the pinned keys; forward secrecy, so a key stolen later
  doesn't open old traffic.
- **Trusted networks only**: home and work Wi-Fi you mark as trusted; on
  public Wi-Fi it stays off unless you allow it (or uses Bluetooth/USB).
- **Least power**: every feature is a separate permission per device; the
  dangerous ones (remote input, clipboard) are off and ask.
- **A lost phone**: "Forget this phone" on the PC cuts it off at once (its
  key is no longer accepted). The phone's own lock and encryption protect
  what's on it.
- **Written in Rust, confined**: `nexa-linkd` (one per user) parses every
  message with size limits and a strict, versioned format; Landlock limits
  it to its own data and Downloads, the network to its own port; the parser
  is fuzzed (cargo-fuzz) before each release.
- **Nothing in the cloud**: no account, no server, no relay. Works offline.
- **Logged**: Settings > Nexa Link shows what each device did and when.

What it can't protect against: someone holding your **unlocked** phone, or
a bug we haven't found (hence Rust, fuzzing, and features off by default).

## 3. How it is built

- **`nexa-linkd` (Rust)**: device keys (`~/.local/share/nexa-link`, 0600),
  pairing, discovery (mDNS on the trusted network; Bluetooth LE on the phone),
  the Noise channel (the `snow` crate), message routing, the per-device
  permission table, the log. JSON lines to the UI, like the other Nexa
  services.
- **Settings > Nexa Link** (thin UI): pair (QR), devices, permissions, trusted
  networks, forget a device.
- **Integrations**: notifications into mako; calls/texts through the phone's
  ModemManager; files through nexa-fileops (never overwrite); the top bar
  and the Dock show the phone.

## 4. Phases

| Phase | What | Done when |
|---|---|---|
| L1 | Keys, QR pairing, Noise channel, discovery with rotating codes; PC to PC first (two QEMU machines) | an unpaired device and a replayed or forged packet are refused (tests) |
| L2 | Files, links, find my phone, battery | headless test between two machines |
| L3 | Notifications and replies, media controls | |
| L4 | Calls and texts from the PC (with Nexa Mobile P3) | a call answered on the PC |
| L5 | Clipboard and remote input (opt-in), continue where you were | |
| L6 | Fuzzing in CI, an outside security review before calling it done | |
