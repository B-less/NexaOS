"""Run with: python3 common/tests/test_vpn.py

WireGuard file checks (nexa_common.wgconf) and the Settings helper's VPN
actions, with wg-quick and ip faked and every path in a temp dir."""
import base64, json, os, stat, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "settings-app"))
from nexa_common import wgconf
from nexa_settings import helper

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

# ---- helper actions
tmp = tempfile.mkdtemp()
helper.WG_DIR = os.path.join(tmp, "wireguard")
helper.RESOLV = os.path.join(tmp, "resolv.conf")
helper.RESOLV_BACKUP = os.path.join(tmp, "run", "resolv.conf.before-vpn")
open(helper.RESOLV, "w").write("nameserver 192.168.1.1\n")
up, calls = set(), []

class R:
    def __init__(self, rc=0, out="", err=""): self.returncode, self.stdout, self.stderr = rc, out, err

def fake_run(argv, **kw):
    calls.append(argv)
    if argv[:2] == ["ip", "-o"]:
        return R(out="".join(f"{i+7}: {n}: <POINTOPOINT,UP> mtu 1420\n" for i, n in enumerate(sorted(up))))
    if argv[0] == "wg-quick":
        (up.add if argv[1] == "up" else up.discard)(argv[2])
        return R()
    raise AssertionError(argv)
helper._run = fake_run

check("import: bad name refused", not helper.vpn_import({"name": "a/b", "config": GOOD})["ok"])
check("import: unsafe file refused, nothing written",
      not helper.vpn_import({"name": "evil", "config": GOOD.replace("[Peer]", "PostUp = id\n[Peer]")})["ok"]
      and not os.path.exists(os.path.join(helper.WG_DIR, "evil.conf")))
check("import: ok", helper.vpn_import({"name": "sweden", "config": GOOD})["ok"])
conf = os.path.join(helper.WG_DIR, "sweden.conf")
check("import: file is root-only (0600) and has no DNS line",
      stat.S_IMODE(os.stat(conf).st_mode) == 0o600 and "DNS" not in open(conf).read())
helper.vpn_import({"name": "home", "config": GOOD.replace("DNS = 10.2.0.1, vpn.example\n", "")})
check("list", helper.vpn_list({})["tunnels"] == [{"name": "home", "active": False}, {"name": "sweden", "active": False}])

check("up: ok", helper.vpn_up({"name": "sweden"})["ok"] and up == {"sweden"})
check("up: VPN DNS in use, old resolv.conf kept aside",
      open(helper.RESOLV).read().endswith("nameserver 10.2.0.1\n") and "192.168.1.1" in open(helper.RESOLV_BACKUP).read())
check("up: switching VPN takes the other down first", helper.vpn_up({"name": "home"})["ok"] and up == {"home"})
check("up: unknown tunnel refused", not helper.vpn_up({"name": "nope"})["ok"])
helper.vpn_up({"name": "sweden"})
check("down: tunnel down, original DNS back",
      helper.vpn_down({"name": "sweden"})["ok"] and not up and open(helper.RESOLV).read() == "nameserver 192.168.1.1\n"
      and not os.path.exists(helper.RESOLV_BACKUP))
helper.vpn_up({"name": "sweden"})
check("remove: takes it down and deletes the files",
      helper.vpn_remove({"name": "sweden"})["ok"] and not up and not os.path.exists(conf)
      and [t["name"] for t in helper.vpn_list({})["tunnels"]] == ["home"])

# ---- rfkill
helper.RFKILL_DIR = os.path.join(tmp, "rfkill")
for dev, kind, hard in (("rfkill0", "wlan", "0"), ("rfkill1", "bluetooth", "0")):
    os.makedirs(os.path.join(helper.RFKILL_DIR, dev))
    for f, v in (("type", kind), ("soft", "1"), ("hard", hard)):
        open(os.path.join(helper.RFKILL_DIR, dev, f), "w").write(v)
check("bluetooth unblock: only the Bluetooth radio",
      helper.bluetooth_unblock({})["ok"] and open(os.path.join(helper.RFKILL_DIR, "rfkill1", "soft")).read() == "0"
      and open(os.path.join(helper.RFKILL_DIR, "rfkill0", "soft")).read() == "1")
open(os.path.join(helper.RFKILL_DIR, "rfkill1", "hard"), "w").write("1")
check("bluetooth unblock: hardware switch reported", "hardware" in helper.bluetooth_unblock({}).get("error", ""))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
