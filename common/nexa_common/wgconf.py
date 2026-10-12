"""Checking WireGuard tunnel files before they are installed.

A .conf from a VPN provider is handed to `wg-quick` as root, and wg-quick
runs PreUp/PostUp/PreDown/PostDown lines as shell commands. So imports are
strict: only the plain tunnel settings below are accepted, anything else is
refused with a reason. DNS is taken out and applied by the Settings helper
itself (wg-quick would need resolvconf, which NexaOS doesn't use).
"""
import base64
import ipaddress
import re

NAME_RE = re.compile(r"^[a-zA-Z0-9_=+.-]{1,15}$")  # Linux interface-name rules
INTERFACE_KEYS = {"privatekey", "address", "listenport", "dns", "mtu", "table", "fwmark"}
PEER_KEYS = {"publickey", "presharedkey", "allowedips", "endpoint", "persistentkeepalive"}
SCRIPT_KEYS = {"preup", "postup", "predown", "postdown", "saveconfig"}
CANONICAL = {k.lower(): k for k in ("PrivateKey", "Address", "ListenPort", "DNS", "MTU", "Table", "FwMark",
                                    "PublicKey", "PresharedKey", "AllowedIPs", "Endpoint", "PersistentKeepalive")}


class WgConfError(ValueError):
    pass


def _key(value, what):
    try:
        raw = base64.b64decode(value, validate=True)
    except ValueError:
        raw = b""
    if len(raw) != 32:
        raise WgConfError(f"{what} isn't a valid WireGuard key.")


def _addresses(value, what):
    for part in value.split(","):
        try:
            ipaddress.ip_interface(part.strip())
        except ValueError:
            raise WgConfError(f"{what}: '{part.strip()}' isn't an IP address.")


def name_from_filename(filename):
    """'My VPN (Sweden).conf' -> 'My-VPN-Sweden' (fits an interface name)."""
    stem = re.sub(r"\.conf$", "", filename.rsplit("/", 1)[-1], flags=re.I)
    name = re.sub(r"[^a-zA-Z0-9_=+.-]+", "-", stem).strip("-.")[:15].strip("-.")
    return name or "vpn"


def validate(text):
    """Returns (clean_config_text, dns_servers). Raises WgConfError."""
    sections = []
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = re.fullmatch(r"\[(\w+)\]", line)
        if m:
            kind = m.group(1).lower()
            if kind not in ("interface", "peer"):
                raise WgConfError(f"Line {n}: unknown section [{m.group(1)}].")
            sections.append((kind, []))
            continue
        if "=" not in line or not sections:
            raise WgConfError(f"Line {n} isn't a WireGuard setting.")
        key, value = (s.strip() for s in line.split("=", 1))
        if key.lower() in SCRIPT_KEYS:
            raise WgConfError(f"This file runs commands ({key}), which NexaOS doesn't allow in VPN files.")
        allowed = INTERFACE_KEYS if sections[-1][0] == "interface" else PEER_KEYS
        if key.lower() not in allowed:
            raise WgConfError(f"Line {n}: '{key}' isn't allowed in [{sections[-1][0].title()}].")
        sections[-1][1].append((key.lower(), value))

    interfaces = [s for k, s in sections if k == "interface"]
    peers = [s for k, s in sections if k == "peer"]
    if len(interfaces) != 1:
        raise WgConfError("A WireGuard file needs exactly one [Interface] section.")
    if not peers:
        raise WgConfError("A WireGuard file needs at least one [Peer] (the VPN server).")
    iface = dict(interfaces[0])
    if "privatekey" not in iface or "address" not in iface:
        raise WgConfError("[Interface] needs a PrivateKey and an Address.")
    _key(iface["privatekey"], "PrivateKey")
    _addresses(iface["address"], "Address")
    for p in peers:
        d = dict(p)
        if "publickey" not in d:
            raise WgConfError("Every [Peer] needs a PublicKey.")
        _key(d["publickey"], "PublicKey")
        if "presharedkey" in d:
            _key(d["presharedkey"], "PresharedKey")
        if "allowedips" in d:
            _addresses(d["allowedips"], "AllowedIPs")
        if "endpoint" in d and not re.fullmatch(r"(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9.-]+):\d{1,5}", d["endpoint"]):
            raise WgConfError(f"Endpoint '{d['endpoint']}' should look like host:port.")

    dns = []
    out = []
    for kind, items in sections:
        out.append(f"[{kind.title()}]")
        for key, value in items:
            if key == "dns":
                for part in value.split(","):
                    part = part.strip()
                    try:
                        dns.append(str(ipaddress.ip_address(part)))
                    except ValueError:
                        pass  # search domains: ignored
                continue
            out.append(f"{CANONICAL[key]} = {value}")
        out.append("")
    return "\n".join(out), dns
