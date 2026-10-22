#!/usr/bin/env python3
"""Minimal MCP client for figcraft-cli: join the live doc, run tool calls from stdin.

Usage: EDIT_LINK='figcraft://...' python3 call.py < calls.json
calls.json: [{"name": "get_node", "arguments": {"id": 35, "depth": 2}}, ...]
Prints one JSON result per call, tagged with its index.
"""
import json
import os
import queue
import subprocess
import sys
import threading
import time

CLI = "/root/code/mindmap/server/figcraft/target/release/figcraft-cli"
LINK = os.environ["EDIT_LINK"]

proc = subprocess.Popen(
    [CLI, "mcp"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1,
)
lines: "queue.Queue[str]" = queue.Queue()


def pump(stream, sink):
    for line in stream:
        sink.put(line)


threading.Thread(target=pump, args=(proc.stdout, lines), daemon=True).start()


def send(obj):
    proc.stdin.write(json.dumps(obj) + "\n")
    proc.stdin.flush()


def call(msg_id, name, arguments=None, timeout=60):
    if name is not None:
        send({"jsonrpc": "2.0", "id": msg_id, "method": "tools/call",
              "params": {"name": name, "arguments": arguments or {}}})
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            raw = lines.get(timeout=max(0.1, deadline - time.time()))
        except queue.Empty:
            break
        try:
            msg = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if msg.get("id") == msg_id:
            return msg
        # otherwise a notification or stale response: ignore
    raise TimeoutError(f"no response for id {msg_id} ({name})")


send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                 "clientInfo": {"name": "mindmap-agent", "version": "0"}}})
init = call(1, None)
send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

joined = call(2, "collab.join", {"link": LINK})
if "error" in joined:
    print(json.dumps({"step": "join", "error": joined["error"]}), flush=True)

# wait for the session to go live
peers = None
for _ in range(40):
    time.sleep(0.25)
    peers = call(3, "collab.peers", {})
    try:
        text = peers["result"]["content"][0]["text"]
        data = json.loads(text)
    except Exception:
        continue
    sessions = data.get("sessions") or [data]
    if any(s.get("status") == "live" for s in sessions):
        break
print(json.dumps({"step": "peers", "status": [
    s.get("status") for s in (peers or {}).get("result", {}).get("content", [])][:0] or "ok"}), flush=True)

calls = json.load(sys.stdin)
for index, item in enumerate(calls):
    response = call(100 + index, item["name"], item.get("arguments"))
    result = response.get("result", response)
    content = result.get("content") if isinstance(result, dict) else None
    if isinstance(content, list) and content and content[0].get("type") == "text":
        text = content[0]["text"]
        try:
            parsed = json.loads(text)
            print(json.dumps({"call": index, "name": item["name"], "ok": not result.get("isError"),
                              "data": parsed}), flush=True)
        except json.JSONDecodeError:
            print(json.dumps({"call": index, "name": item["name"], "ok": not result.get("isError"),
                              "text": text}), flush=True)
    else:
        print(json.dumps({"call": index, "name": item["name"], "raw": result}), flush=True)

proc.stdin.close()
proc.wait(timeout=10)
