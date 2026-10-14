from . import client, tools

SYSTEM_PROMPT = """You are the built-in system assistant for NexaOS, running as a local process \
on the machine you are inspecting. You have tools to read real hardware and kernel state (CPU, \
memory, temperatures, PCI/USB devices, disks, kernel modules, logs); to browse, search, read, \
write, move, copy and trash files in the user's home, tidy folders, find duplicate files, and \
undo any of those file actions (every one gets an undo_id); to list \
installed apps and launch them; to list, focus, and close open windows in the current Sway \
session; to search installed apk packages and see what is installed; to read and change the \
system volume and mute state; to see, connect to, disconnect from, and forget WiFi networks (never handling passwords - new secured networks are set up in the Network app); to show desktop notifications; and, when control is enabled, to \
act on the machine itself (change CPU governor, \
start/stop/restart services, load/unload kernel modules, set hostname, reboot, power off, \
install or remove apk packages).

Always prefer calling a tool over guessing. Report real numbers and real file/window/package \
listings from tool results, not estimates. Before calling a tool that changes something (writing, \
moving, or deleting a file, launching an app, closing a window, or a control action), briefly say \
what you are about to do and why. For tidying, call organize_folder, \
show the plan, and call apply_moves only after the user agrees. Never call reboot_system, \
poweroff_system, delete_file, or remove_package without the user explicitly asking for it in this conversation."""

LOCAL_SYSTEM_PROMPT = """You are Nexa, the assistant built into NexaOS, running on this computer \
with no internet needed. You can look at and change the user's files (home folders: ~/Desktop, \
~/Documents, ~/Downloads, ~/Music, ~/Pictures, ~/Videos), tidy folders and find duplicates, check \
the network, CPU, memory, disk and temperatures, change the volume, open apps, and undo your file \
changes. Use the tools to get real facts instead of guessing. If a tool says a path doesn't exist, \
read the list of what is there and try the right name yourself before asking the user. Say briefly \
what you are about to do before changing anything; every file change has an Undo button for the \
user. To tidy a folder, call organize_folder, show the plan, and call apply_moves only after the \
user agrees. Answer in a few short sentences."""

MAX_TURNS = 8


def run(api_key, model, user_query, allow_control, require_confirmation, confirm_fn, log_fn,
        on_event, history=None, backend="claude"):
    """Runs the agent loop, emitting on_event(kind, payload) as it goes instead of
    printing directly, so CLI and GUI callers can render each kind their own way.

    Event kinds: "text" (payload: str), "tool_call" (payload: {name, args}),
    "tool_result" (payload: {name, args, result, error}).
    """
    """backend: "claude" (online) or "local" (Nexa Mind, on this machine)."""
    messages = list(history or [])
    messages.append({"role": "user", "content": user_query})
    if backend == "local":
        from . import local_client
        said = [m["content"] for m in messages if m["role"] == "user" and isinstance(m["content"], str)]
        send, system, specs = (local_client.create_message, LOCAL_SYSTEM_PROMPT,
                               tools.anthropic_tool_specs(tools.local_tools_for(said)))
    else:
        send, system, specs = client.create_message, SYSTEM_PROMPT, tools.anthropic_tool_specs()

    for _ in range(MAX_TURNS):
        resp = send(api_key, model, system, messages, specs)
        content = resp.get("content", [])
        stop = resp.get("stop_reason")
        if stop == "refusal":
            # Nothing usable came back; keep it out of the history so the
            # next question starts from a valid conversation.
            reason = (resp.get("stop_details") or {}).get("explanation")
            on_event("text", "Claude declined this request." + (f" ({reason})" if reason else ""))
            return messages
        # Passed back unchanged (thinking blocks included) on the next turn.
        messages.append({"role": "assistant", "content": content})

        text_parts = [b["text"] for b in content if b.get("type") == "text"]
        tool_uses = [b for b in content if b.get("type") == "tool_use"]

        for t in text_parts:
            on_event("text", t)

        if not tool_uses:
            if stop == "max_tokens":
                on_event("text", "(The reply was cut off at the length limit.)")
            return messages

        tool_results = []
        for call in tool_uses:
            name, args, call_id = call["name"], call.get("input", {}), call["id"]
            spec = tools.TOOLS.get(name)
            on_event("tool_call", {"name": name, "args": args})
            needs_confirm = spec and spec["category"] in (tools.CONTROL, tools.ACTION) and (
                require_confirmation or name in tools.ALWAYS_CONFIRM
            )
            if needs_confirm and not confirm_fn(name, args):
                log_fn(name, args, None, error="declined by user")
                on_event("tool_result", {"name": name, "args": args, "result": None,
                                          "error": "declined by user"})
                tool_results.append({
                    "type": "tool_result", "tool_use_id": call_id,
                    "content": "user declined to confirm this action", "is_error": True,
                })
                continue
            try:
                result = tools.run_tool(name, args, allow_control)
                log_fn(name, args, result, error=None)
                on_event("tool_result", {"name": name, "args": args, "result": result, "error": None})
                tool_results.append({
                    "type": "tool_result", "tool_use_id": call_id,
                    "content": _stringify(result),
                })
            except tools.ToolError as e:
                log_fn(name, args, None, error=str(e))
                on_event("tool_result", {"name": name, "args": args, "result": None, "error": str(e)})
                tool_results.append({
                    "type": "tool_result", "tool_use_id": call_id,
                    "content": f"error: {e}", "is_error": True,
                })

        messages.append({"role": "user", "content": tool_results})

    on_event("text", "(stopped after reaching the tool-call limit for this turn)")
    return messages


def _stringify(result):
    if isinstance(result, str):
        return result
    import json
    return json.dumps(result, indent=2)
