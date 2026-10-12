import json
import urllib.request
import urllib.error

API_URL = "https://api.anthropic.com/v1/messages"
API_VERSION = "2023-06-01"
# Server-side refusal fallback: if Claude's safety classifiers decline a
# request, the API re-runs it on Anthropic's recommended fallback model
# (picked by refusal category) instead of returning the refusal.
FALLBACK_BETA = "server-side-fallback-2026-07-01"


class ApiError(Exception):
    pass


class NetworkError(Exception):
    pass


def create_message(api_key, model, system, messages, tools, max_tokens=16000, timeout=120):
    # Current models think before answering and thinking counts toward
    # max_tokens, so 1024 cut replies off; requests also take longer.
    body = json.dumps({
        "model": model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": messages,
        "tools": tools,
        "fallbacks": "default",
    }).encode("utf-8")

    req = urllib.request.Request(
        API_URL, data=body, method="POST",
        headers={
            "content-type": "application/json",
            "x-api-key": api_key,
            "anthropic-version": API_VERSION,
            "anthropic-beta": FALLBACK_BETA,
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", errors="replace")
        raise ApiError(f"HTTP {e.code}: {detail}")
    except urllib.error.URLError as e:
        raise NetworkError(str(e.reason))
    except TimeoutError:
        raise NetworkError("request timed out")
