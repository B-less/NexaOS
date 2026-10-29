# Nexa Link: your PC and your phone, connected (design, 2026-10-25)

NexaOS on the PC and Nexa Mobile (docs/MOBILE_PLAN.md) find each other and
connect **automatically**, but only after you have paired them once, only
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
| Shared clipboard | **off**, and never anything marked as a password |
| Use the phone as a remote / touchpad, or the PC's keyboard on the phone | **off**; asks every session |
| Continue where you were (open the page or file the other device has open) | on |

## 2. Is "automatic" safe? Yes, if these hold

Automatic means: two devices **you already paired** connect by themselves
when they are near (same trusted network, or Bluetooth). A device you never
paired never gets anything. KDE Connect, the best-known Linux-phone link,
has had security holes; here is each one and the Nexa Link rule against it:

| Lesson (KDE Connect) | Nexa Link rule |
|---|---|
| An unpaired device could connect: the identity sent before encryption wasn't checked against the one inside it ([CVE-2025-66270](https://www.sentinelone.com/vulnerability-database/cve-2025-66270/)) | A device **is** its public key (Ed25519). There is no separate "device ID" to trust: the encrypted handshake proves the key, and only keys pinned at pairing are accepted. Unknown keys are dropped before a single message is read |
| The pairing code was short and fixed, so an attacker could make keys until the code matched ([CVE-2025-32898](https://kde.org/info/security/advisory-20250418-3.txt)) | Pairing by **QR code** (the PC shows its key; the phone scans it) or a code that mixes **fresh random values from both devices** (it changes every attempt), confirmed on both screens within 60 s |
| Unauthenticated discovery packets could fake names or unpair devices ([CVE-2025-32899](https://nvd.nist.gov/vuln/detail/cve-2025-32899), [CVE-2025-32900](https://kde.org/info/security/advisory-20250418-2.txt)) | Discovery messages **change nothing**. They only say "a paired device may be here", as a rotating code only your paired devices can recognise (strangers on the network can't even tell it's you). Names, unpairing and everything else happen only inside the encrypted channel |
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

## 2b. Nexa-only signals, and Nexa Drop

Nexa Link talks only to Nexa. On the local network each device sends a
short signal every 3 s (to each network's broadcast address, worked out
from the address and netmask, and straight to where each paired device was
last heard, since many Wi-Fi routers hold back broadcasts between devices): 32 bytes, 16 random and a 16-byte tag made with a
key the two devices agreed on when they paired (HMAC-SHA256 over a
5-minute window, the sender's public key and the random part). A paired
device recognises it as "my PC is here". To any other phone, laptop or
sniffer it is random bytes that never repeat: no name, no device id, no
fixed MAC-like value to follow someone with. A recorded signal stops
working after a few minutes.

**Nexa Drop** sends to a Nexa device you haven't paired with, like
handing over a photo to someone nearby. The receiver turns on "Receive
from any Nexa device" for a few minutes (off again by itself). Its signal
then uses a key every Nexa device knows, so nearby Nexa devices list it;
to everything else it's still random bytes. Who it is comes only inside
the encrypted channel, and every item is asked on screen ("Photo from
Kofi's phone: Save / Decline"); an unpaired device can never send without
that question. Honest limit: anyone running NexaOS's (open) code can tell
that *a* device is taking drops while it is on; with it off, only paired
devices know you're there.

**The pairing code** is `NEXA-LINK:1:<addresses>:<secret>`: every local
IPv4 address of the device showing it (up to 4; a PC on Wi-Fi and a cable,
or with a VPN, has several and only one may be on the other device's
network), then 16 random bytes. The device that types or scans it tries the
addresses on its own networks first. Until 2026-10-29 the code carried only
the address of the default route, and two PCs with a second network never
paired; nor did they find each other where the broadcast address wasn't
set. Checked since by `scripts/test-nexa-link-vms.py`: two NexaOS VMs, a
private network plus an internet one each, pair, find each other and send
a file through the real firewall, sandbox and session.

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
| L1 | Keys, QR pairing, Noise channel, discovery with rotating codes, files with Nexa Drop; PC to PC first | **built (2026-10-25)**: `nexa-linkd` (Rust), `make test-nexa-link` pairs a PC and a phone on a private network with a stranger listening: same six digits on both, signals random to the stranger, files asked and checksummed, the stranger refused, Drop asked first, Forget cuts off at once |
| L2 | Files, links, find my phone, battery | files, links and Ring **built (2026-10-25)** with their screens (Settings > Nexa Link, pop-ups, Files > Send to a Device), `make test-nexa-link-ui`; battery waits for Nexa Mobile |
| L3 | Notifications and replies, media controls | |
| L4 | Calls and texts from the PC (with Nexa Mobile P3) | a call answered on the PC |
| L5 | Clipboard and remote input (opt-in), continue where you were | |
| L6 | Fuzzing in CI, an outside security review before calling it done | |
