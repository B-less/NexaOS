"""What the address bar does with what you type. No GTK, so it can be tested."""
import os
import re
from urllib.parse import quote

HOME_URL = "https://duckduckgo.com/"
SEARCH_URL = "https://duckduckgo.com/?q="
# Addresses that are complete as typed (no "https://" to add).
_SCHEME_ONLY = ("about:", "data:", "file:", "view-source:", "mailto:")
_HOST_PORT = re.compile(r"^(localhost|[\w.-]+\.[a-z]{2,}|\d{1,3}(\.\d{1,3}){3})(:\d+)?(/.*)?$", re.I)


def to_uri(text):
    """'example.com' -> https://example.com, 'about:blank' stays, a local
    path becomes a file:// URI, anything else is a DuckDuckGo search."""
    text = text.strip()
    if not text:
        return HOME_URL
    if "://" in text.split(" ", 1)[0] and " " not in text:
        return text
    if text.lower().startswith(_SCHEME_ONLY) and " " not in text:
        return text
    if text.startswith(("/", "~/")) and os.path.exists(os.path.expanduser(text)):
        return "file://" + quote(os.path.abspath(os.path.expanduser(text)))
    if " " not in text and _HOST_PORT.match(text):
        scheme = "http://" if text.lower().startswith("localhost") or re.match(r"^\d", text) else "https://"
        return scheme + text
    return SEARCH_URL + quote(text, safe="")
