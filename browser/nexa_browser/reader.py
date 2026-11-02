"""Reader view's page: the article nexa-webdata found, in the theme's
colours, with no scripts (a Content-Security-Policy allows only images and
this page's own style). No GTK here, so it can be unit-tested."""
import html
from urllib.parse import urlsplit

BASE = "about:reader"

DEFAULT_COLORS = {"bg": "#0c1228", "surface": "#101732", "text": "#d9e0f6", "muted": "#8f9bc4", "accent": "#4da3ff"}


def page(article, url, colors=None):
    c = dict(DEFAULT_COLORS)
    c.update({k: v for k, v in (colors or {}).items() if isinstance(v, str) and v.startswith("#") and len(v) <= 9})
    site = urlsplit(url).hostname or ""
    bits = [b for b in (html.escape(article.get("byline") or ""), html.escape(site), f"{article.get('minutes', 1)} min read") if b]
    original = html.escape(url) if url.startswith(("https://", "http://")) else ""
    return f"""<!DOCTYPE html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src https: http: data:; style-src 'unsafe-inline'">
<meta name="referrer" content="no-referrer">
<title>{html.escape(article.get('title') or site)}</title>
<style>
:root {{ color-scheme: dark light; }}
body {{ margin: 0; background: {c['bg']}; color: {c['text']}; font: 19px/1.7 "Noto Serif", Georgia, serif; }}
main {{ max-width: 68ch; margin: 0 auto; padding: 48px 24px 96px; }}
h1 {{ font: 700 2rem/1.25 Inter, sans-serif; margin: 0 0 8px; }}
h2, h3, h4, h5, h6 {{ font-family: Inter, sans-serif; line-height: 1.3; margin: 1.6em 0 .4em; }}
.meta {{ color: {c['muted']}; font: 15px Inter, sans-serif; margin-bottom: 32px; }}
.meta a {{ color: {c['accent']}; }}
a {{ color: {c['accent']}; }}
img {{ max-width: 100%; height: auto; border-radius: 10px; display: block; margin: 16px auto; }}
figcaption {{ color: {c['muted']}; font-size: .85em; text-align: center; }}
blockquote {{ border-left: 4px solid {c['accent']}; margin: 1em 0; padding: 0 0 0 16px; color: {c['muted']}; }}
pre, code {{ font-family: "JetBrains Mono", monospace; font-size: .85em; background: {c['surface']}; border-radius: 6px; }}
pre {{ padding: 12px 16px; overflow-x: auto; }}
table {{ border-collapse: collapse; }} td, th {{ border: 1px solid {c['muted']}; padding: 4px 8px; }}
hr {{ border: none; border-top: 1px solid {c['muted']}; }}
</style></head><body><main>
<h1>{html.escape(article.get('title') or site)}</h1>
<div class="meta">{' · '.join(bits)}{f' · <a href="{original}">Original page</a>' if original else ''}</div>
{article.get('html', '')}
</main></body></html>"""
