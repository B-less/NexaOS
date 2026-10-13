"""The assistant's on-device back-end: talks to nexa-mindd (Rust, see
tools/nexa-mindd) over its Unix socket.

create_message() has the same signature and return shape as
client.create_message() (Anthropic's Messages format), so agent.run() and
every tool work unchanged; this module translates to and from the
OpenAI-style chat format the local model server speaks, including tool
calls and tool results.
"""
import json
import uuid

from nexa_common import mind
from .client import ApiError, NetworkError

# A laptop CPU reads prompts at ~40 tokens/s (docs/NEXA_MIND.md), so the
# local model gets a short system prompt and only the most useful tools.
LOCAL_MAX_TOKENS = 1024


def _request(method, path, body=None, timeout=600):
    try:
        return mind.request(method, path, body, timeout)
    except mind.MindUnreachable as e:
        raise NetworkError(str(e))
    except mind.MindError as e:
        raise ApiError(str(e))


status, available, models, download = mind.status, mind.available, mind.models, mind.download


def to_openai(system, messages, tools):
    """Anthropic-format conversation -> OpenAI chat messages and tools."""
    out = [{"role": "system", "content": system}]
    for m in messages:
        content = m["content"]
        if isinstance(content, str):
            out.append({"role": m["role"], "content": content})
            continue
        if m["role"] == "assistant":
            text = "".join(b.get("text", "") for b in content if b.get("type") == "text")
            calls = [{"id": b["id"], "type": "function",
                      "function": {"name": b["name"], "arguments": json.dumps(b.get("input") or {})}}
                     for b in content if b.get("type") == "tool_use"]
            msg = {"role": "assistant", "content": text}
            if calls:
                msg["tool_calls"] = calls
            out.append(msg)
            continue
        for b in content:  # user turn: tool results and/or text
            if b.get("type") == "tool_result":
                body = b.get("content")
                out.append({"role": "tool", "tool_call_id": b["tool_use_id"],
                            "content": body if isinstance(body, str) else json.dumps(body)})
            elif b.get("type") == "text":
                out.append({"role": "user", "content": b["text"]})
    specs = [{"type": "function", "function": {"name": t["name"], "description": t["description"],
                                               "parameters": t["input_schema"]}} for t in tools]
    return out, specs


def from_openai(resp):
    """OpenAI chat completion -> Anthropic-format response dict."""
    choice = (resp.get("choices") or [{}])[0]
    msg = choice.get("message") or {}
    content = []
    if msg.get("content"):
        content.append({"type": "text", "text": msg["content"]})
    for call in msg.get("tool_calls") or []:
        fn = call.get("function") or {}
        try:
            args = json.loads(fn.get("arguments") or "{}")
        except ValueError:
            args = {}
        content.append({"type": "tool_use", "id": call.get("id") or f"call_{uuid.uuid4().hex[:12]}",
                        "name": fn.get("name", ""), "input": args if isinstance(args, dict) else {}})
    has_tools = any(b["type"] == "tool_use" for b in content)
    finish = choice.get("finish_reason")
    stop = "tool_use" if has_tools else "max_tokens" if finish == "length" else "end_turn"
    return {"content": content, "stop_reason": stop}


def create_message(api_key, model, system, messages, tools, max_tokens=LOCAL_MAX_TOKENS, timeout=600):
    """Same contract as client.create_message; api_key is ignored (the
    socket's permissions and the service's peer check do the access control)."""
    oa_messages, oa_tools = to_openai(system, messages, tools)
    body = {"messages": oa_messages, "max_tokens": min(max_tokens, LOCAL_MAX_TOKENS), "temperature": 0.3}
    if oa_tools:
        body["tools"] = oa_tools
    if model:
        body["model"] = model
    return from_openai(_request("POST", "/v1/chat/completions", body, timeout=timeout))
