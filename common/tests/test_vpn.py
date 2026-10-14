"""Run with: python3 common/tests/test_vpn.py

WireGuard file checks (nexa_common.wgconf, used by Settings to name and
pre-check files; the broker re-checks everything in Rust)."""
import base64, json, os, stat, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "settings-app"))
from nexa_common import wgconf

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

KEY = base64.b64encode(b"k" * 32).decode()
GOOD = f"""# from the provider
[Interface]
PrivateKey = {KEY}
Address = 10.2.0.2/32, fd00::2/128
DNS = 10.2.0.1, vpn.example

[Peer]
PublicKey = {KEY}
AllowedIPs = 0.0.0.0/0, ::/0
Endpoint = se.vpn.example:51820
PersistentKeepalive = 25
"""

def err(text):
    try:
        wgconf.validate(text)
    except wgconf.WgConfError as e:
        return str(e)
    return None

clean, dns = wgconf.validate(GOOD)
check("valid file: DNS pulled out, search domain dropped", dns == ["10.2.0.1"], str(dns))
check("valid file: DNS line removed, rest kept", "DNS" not in clean and "Endpoint = se.vpn.example:51820" in clean, clean)
check("cleaned file validates again", wgconf.validate(clean)[1] == [])
check("PostUp refused (would run as root)", "runs commands" in (err(GOOD.replace("[Peer]", "PostUp = curl x|sh\n[Peer]")) or ""))
check("PreDown in lower case refused too", "runs commands" in (err(GOOD.replace("MTU", "").replace("[Peer]", "predown = rm -rf /\n[Peer]")) or ""))
check("unknown keys refused", "isn't allowed" in (err(GOOD.replace("[Peer]", "Foo = 1\n[Peer]")) or ""))
check("no peer refused", "[Peer]" in (err(GOOD.split("[Peer]")[0]) or ""))
check("bad key refused", "valid WireGuard key" in (err(GOOD.replace(f"PublicKey = {KEY}", "PublicKey = abc")) or ""))
check("bad address refused", "isn't an IP" in (err(GOOD.replace("10.2.0.2/32", "ten.two")) or ""))
check("bad endpoint refused", "host:port" in (err(GOOD.replace(":51820", "")) or ""))
check("IPv6 endpoint ok", err(GOOD.replace("se.vpn.example:51820", "[2001:db8::1]:51820")) is None)
check("names from file names", (wgconf.name_from_filename("My VPN (Sweden).conf"), wgconf.name_from_filename("/x/wg0.conf"),
                                wgconf.name_from_filename("((.conf")) == ("My-VPN-Sweden", "wg0", "vpn"))

# The VPN actions themselves (import, list, up/down with DNS, remove) and
# the Bluetooth unblock moved to the Rust system broker: see
# tools/nexa-brokerd/src/vpn.rs and worker.rs tests.

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
