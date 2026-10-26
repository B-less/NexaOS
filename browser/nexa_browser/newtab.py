"""The new-tab page: a search box, your most-visited sites and bookmarks.
Plain HTML made here (no remote content). No GTK, so it can be tested."""
from html import escape

from .prefs import SEARCH_ENGINES, site_of


def page(search, top=(), bookmarks=(), colors=None):
    c = {"bg": "#0f1320", "surface": "#1a2033", "text": "#e6e9f2", "muted": "#8a93a6", "accent": "#5b8cff"}
    c.update(colors or {})
    name, url = SEARCH_ENGINES.get(search, SEARCH_ENGINES["duckduckgo"])
    action, _, param = url.partition("?")
    param = param.rstrip("=") or "q"

    def tiles(items):
        out = []
        for it in items:
            title = escape(it.get("title") or site_of(it["url"]))
            letter = escape((site_of(it["url"]) or "?")[:1].upper())
            out.append(f'<a class="tile" href="{escape(it["url"], quote=True)}" title="{escape(it["url"], quote=True)}">'
                       f'<span class="ico">{letter}</span><span class="t">{title}</span></a>')
        return "".join(out)

    sections = ""
    if top:
        sections += f"<h2>Most visited</h2><div class='grid'>{tiles(top)}</div>"
    if bookmarks:
        sections += f"<h2>Bookmarks</h2><div class='grid'>{tiles(bookmarks)}</div>"
    return f"""<!doctype html><html><head><meta charset="utf-8"><title>New tab</title><style>
body {{ margin: 0; background: {c['bg']}; color: {c['text']}; font: 15px Inter, sans-serif; }}
main {{ max-width: 760px; margin: 12vh auto 0; padding: 0 20px; }}
form {{ display: flex; background: {c['surface']}; border-radius: 26px; padding: 4px 6px 4px 20px; }}
input {{ flex: 1; background: none; border: 0; color: {c['text']}; font-size: 17px; padding: 12px 0; outline: none; }}
button {{ background: {c['accent']}; border: 0; color: #fff; border-radius: 22px; padding: 0 20px; font-size: 15px; }}
h2 {{ color: {c['muted']}; font-size: 12px; letter-spacing: .12em; text-transform: uppercase; margin: 34px 0 12px; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(130px, 1fr)); gap: 12px; }}
.tile {{ background: {c['surface']}; border-radius: 14px; padding: 14px 10px; text-decoration: none; color: {c['text']};
        display: flex; flex-direction: column; align-items: center; gap: 8px; }}
.tile:hover {{ outline: 2px solid {c['accent']}; }}
.ico {{ width: 38px; height: 38px; border-radius: 50%; background: {c['accent']}; color: #fff; display: grid; place-items: center;
       font-weight: 700; }}
.t {{ font-size: 12px; max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
</style></head><body><main>
<form action="{escape(action, quote=True)}" method="get"><input name="{escape(param, quote=True)}" autofocus
 placeholder="Search with {escape(name)} or type an address"><button>Search</button></form>
{sections}</main></body></html>"""
