from . import tools

# No cloud connection and no local LLM yet (see NexaOS docs: real offline reasoning via
# llama.cpp + a bundled GGUF model is a separate, heavier phase of work). Until then this
# is a plain keyword router over the same read-only tools the cloud agent uses — never
# control actions, since acting on hardware without real reasoning behind it is unsafe.
_ROUTES = [
    (("temp", "thermal", "heat"), "get_temperatures"),
    (("memory", "ram", "swap"), "get_mem_info"),
    (("cpu", "processor", "clock", "governor"), "get_cpu_info"),
    (("disk", "storage", "space", "partition"), "list_block_devices"),
    (("usb",), "list_usb_devices"),
    (("pci", "gpu", "graphics card"), "list_pci_devices"),
    (("module", "driver"), "list_kernel_modules"),
    (("log", "dmesg", "kernel message"), "tail_log"),
    (("uptime", "load"), "get_uptime_load"),
    (("service", "daemon"), "list_services"),
]


def answer(query):
    q = query.lower()
    for keywords, tool_name in _ROUTES:
        if any(k in q for k in keywords):
            try:
                result = tools.run_tool(tool_name, {}, allow_control=False)
            except tools.ToolError as e:
                return f"[offline mode] {tool_name} failed: {e}"
            return f"[offline mode, no LLM — direct {tool_name} lookup]\n{_format(result)}"
    return (
        "[offline mode] No cloud connection and no local model installed yet, so I can "
        "only answer direct hardware questions (cpu, memory, disk, temperature, usb, pci, "
        "modules, services, uptime, logs) by reading the hardware straight from /proc and "
        "/sys — I can't reason about anything beyond that right now."
    )


def _format(result):
    if isinstance(result, (list, dict)):
        import json
        return json.dumps(result, indent=2)
    return str(result)
