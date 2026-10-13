import argparse
import datetime
import json
import os
import sys

from . import agent, client, local_fallback

CONFIG_PATH = "/etc/nexa-ai/config.json"
LOG_PATH = "/var/log/nexa-ai.log"
DEFAULT_MODEL = "claude-opus-5-5"


def load_config():
    cfg = {
        "api_key": os.environ.get("NEXA_AI_API_KEY", ""),
        "model": os.environ.get("NEXA_AI_MODEL", DEFAULT_MODEL),
        "allow_control": False,
        "require_confirmation": True,
        # "auto": Nexa Mind on this machine when a model is installed, Claude
        # online if an API key is set, the simple offline helper otherwise.
        # "local" / "claude" force one.
        "backend": os.environ.get("NEXA_AI_BACKEND", "auto"),
    }
    if os.path.exists(CONFIG_PATH):
        try:
            with open(CONFIG_PATH) as f:
                cfg.update(json.load(f))
        except (OSError, json.JSONDecodeError) as e:
            print(f"warning: could not read {CONFIG_PATH}: {e}", file=sys.stderr)
    return cfg


def pick_backend(cfg):
    """"local", "claude", or None (only the keyword helper is left)."""
    from . import local_client
    want = cfg.get("backend", "auto")
    if want in ("auto", "local") and local_client.available():
        return "local"
    if want in ("auto", "claude") and cfg.get("api_key"):
        return "claude"
    return None


def backend_label(backend):
    return {"local": "Nexa Mind (on this computer)", "claude": "Claude (online)"}.get(
        backend, "offline helper (no model installed, no API key)")


def make_logger():
    def log(name, args, result, error):
        entry = {
            "ts": datetime.datetime.utcnow().isoformat() + "Z",
            "tool": name, "args": args,
            "ok": error is None, "error": error,
        }
        try:
            os.makedirs(os.path.dirname(LOG_PATH), exist_ok=True)
            with open(LOG_PATH, "a") as f:
                f.write(json.dumps(entry) + "\n")
        except OSError:
            pass
    return log


def confirm(name, args):
    arg_str = ", ".join(f"{k}={v}" for k, v in (args or {}).items())
    try:
        reply = input(f"nexa-ai wants to run '{name}({arg_str})' — proceed? [y/N] ")
    except EOFError:
        return False
    return reply.strip().lower() in ("y", "yes")


def print_event(kind, payload):
    if kind == "text":
        print(payload)
    elif kind == "tool_call":
        arg_str = ", ".join(f"{k}={v}" for k, v in (payload["args"] or {}).items())
        print(f"  → {payload['name']}({arg_str})")
    elif kind == "tool_result" and payload["error"]:
        print(f"    ! {payload['error']}")


def ask(cfg, query, allow_control_override=None):
    allow_control = cfg["allow_control"] if allow_control_override is None else allow_control_override
    backend = pick_backend(cfg)
    if backend is None:
        print(local_fallback.answer(query))
        return
    try:
        agent.run(
            cfg["api_key"], None if backend == "local" else cfg["model"], query, allow_control,
            cfg["require_confirmation"], confirm, make_logger(), print_event, backend=backend,
        )
    except (client.NetworkError, client.ApiError) as e:
        print(f"({backend_label(backend)} unavailable: {e})")
        print(local_fallback.answer(query))


def main(argv=None):
    parser = argparse.ArgumentParser(prog="nexa-ai", description="NexaOS built-in system assistant")
    parser.add_argument("query", nargs="*", help="one-shot question; omit for an interactive session")
    parser.add_argument("--allow-control", action="store_true", help="allow control actions for this invocation")
    args = parser.parse_args(argv)

    cfg = load_config()
    allow_control = True if args.allow_control else None

    if args.query:
        ask(cfg, " ".join(args.query), allow_control)
        return

    mode = backend_label(pick_backend(cfg))
    control = "on" if (cfg["allow_control"] or args.allow_control) else "off"
    print(f"NexaOS assistant — {mode}, control: {control}. Ctrl-D to exit.")
    while True:
        try:
            query = input("nexa-ai> ").strip()
        except (EOFError, KeyboardInterrupt):
            print()
            return
        if not query:
            continue
        ask(cfg, query, allow_control)
