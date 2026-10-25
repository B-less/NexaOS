"""What Nexus's search box finds: apps, Settings pages, a calculator, and
"Ask Nexa" (the assistant). Each result is a dict:
    {kind, title, subtitle, icon, command}
where command is the argv to run. No GTK here, so it can be tested.
"""
import ast
import operator

from nexa_common import desktopapps

# Settings pages: (page argument for nexa-settings, title, extra words people search for)
SETTINGS_PAGES = (
    ("personalization", "Personalization", "theme wallpaper background colour color dark light glass transparency"),
    ("magic", "Magic Layer", "living background animated wallpaper animation effects motion particles circuit"),
    ("bluetooth", "Bluetooth", "headphones mouse keyboard pair speaker"),
    ("vpn", "VPN", "wireguard private network tunnel"),
    ("system", "System", "hostname computer name live usb keep files persistence"),
    ("time", "Time & Keyboard", "clock date timezone time zone language keyboard layout"),
    ("users", "Users", "accounts people account password administrator add person"),
    ("updates", "Updates", "update upgrade software new version"),
    ("startup", "Startup", "services boot start at startup background"),
    ("power", "Power", "battery sleep lock screen idle password"),
    ("privacy", "Privacy", "permissions apps camera microphone internet patterns suggestions learning"),
    ("assistant", "Assistant", "nexa ai claude api key model"),
    ("about", "About", "version system information hardware"),
)

_OPS = {ast.Add: operator.add, ast.Sub: operator.sub, ast.Mult: operator.mul, ast.Div: operator.truediv,
        ast.Pow: operator.pow, ast.Mod: operator.mod, ast.FloorDiv: operator.floordiv,
        ast.USub: operator.neg, ast.UAdd: operator.pos}


def calculate(text):
    """'2+2*3' -> 8; None unless the whole query is plain arithmetic.
    Walks the syntax tree itself: never eval()."""
    expr = text.strip().replace("×", "*").replace("÷", "/").replace("^", "**").replace(",", "")
    if not expr or not any(c.isdigit() for c in expr) or not any(c in "+-*/%" for c in expr):
        return None
    try:
        tree = ast.parse(expr, mode="eval")
    except SyntaxError:
        return None

    def ev(node):
        if isinstance(node, ast.Expression):
            return ev(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
            return node.value
        if isinstance(node, ast.BinOp) and type(node.op) in _OPS:
            left, right = ev(node.left), ev(node.right)
            if isinstance(node.op, ast.Pow) and abs(right) > 100:
                raise ValueError("too big")
            return _OPS[type(node.op)](left, right)
        if isinstance(node, ast.UnaryOp) and type(node.op) in _OPS:
            return _OPS[type(node.op)](ev(node.operand))
        raise ValueError("not arithmetic")
    try:
        value = ev(tree)
    except (ValueError, ZeroDivisionError, OverflowError, TypeError):
        return None
    if isinstance(value, float) and value.is_integer() and abs(value) < 1e15:
        value = int(value)
    return value


def _format_number(v):
    if isinstance(v, float):
        return f"{v:.10g}"
    return f"{v:,}".replace(",", " ") if abs(v) >= 10000 else str(v)


def search(apps, query, limit=12):
    query = query.strip()
    if not query:
        return []
    results = []
    value = calculate(query)
    if value is not None:
        results.append({"kind": "calc", "title": f"= {_format_number(value)}", "subtitle": query,
                        "icon": "accessories-calculator", "command": None, "value": str(value)})
    for app in desktopapps.search(apps, query)[:8]:
        results.append({"kind": "app", "title": app["name"], "subtitle": app["comment"],
                        "icon": app["icon"], "command": desktopapps.command(app["exec"])})
    words = query.lower().split()
    for page, title, keywords in SETTINGS_PAGES:
        hay = f"{title} {keywords} settings".lower()
        if all(w in hay for w in words):
            results.append({"kind": "setting", "title": title, "subtitle": "Settings",
                            "icon": "nexa-settings", "command": ["nexa-settings", page]})
    if value is None:
        results.append({"kind": "ask", "title": f"Ask Nexa: “{query}”",
                        "subtitle": "The assistant answers, or does it for you",
                        "icon": "nexa-ai", "command": ["nexa-ai-gui", "--ask", query]})
    return results[:limit]
