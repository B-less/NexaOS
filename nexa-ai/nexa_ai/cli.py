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
    }
    if os.path.exists(CONFIG_PATH):
        try:
            with open(CONFIG_PATH) as f:
                cfg.update(json.load(f))
        except (OSError, json.JSONDecodeError) as e:
            print(f"warning: could not read {CONFIG_PATH}: {e}", file=sys.stderr)
    return cfg


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
    if not cfg.get("api_key"):
        print(local_fallback.answer(query))
        return
    try:
        agent.run(
            cfg["api_key"], cfg["model"], query, allow_control,
            cfg["require_confirmation"], confirm, make_logger(), print_event,
        )
    except (client.NetworkError, client.ApiError) as e:
        print(f"(cloud AI unavailable: {e})")
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

    mode = "cloud" if cfg.get("api_key") else "offline (no API key configured)"
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
