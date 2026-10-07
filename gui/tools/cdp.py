"""Drive the running Ceps window over the Chrome DevTools Protocol: evaluate
a JS file in the page and print the result plus any console output.

Launch the app with remote debugging first (PowerShell, Windows only):
    $env:CEPS_DEVTOOLS_PORT = "9222"
    .\\target\\debug\\ceps-gui.exe

    python tools/cdp.py tools/probe_speak.js [seconds to wait]
"""
import io
import json
import sys
import time
import urllib.request

import websocket

src = io.open(sys.argv[1], encoding="utf-8").read()
wait = float(sys.argv[2]) if len(sys.argv) > 2 else 5.0

targets = json.load(urllib.request.urlopen("http://127.0.0.1:9222/json"))
page = next(t for t in targets if t.get("type") == "page")
# no Origin header, or Chromium wants --remote-allow-origins
ws = websocket.create_connection(page["webSocketDebuggerUrl"], suppress_origin=True)
seq = 0


def send(method, params=None):
    global seq
    seq += 1
    ws.send(json.dumps({"id": seq, "method": method, "params": params or {}}))
    return seq


send("Runtime.enable")
send("Log.enable")
# userGesture: a scripted click must count as a real one, or play() is refused
want = send("Runtime.evaluate", {"expression": src, "awaitPromise": True, "returnByValue": True,
                                 "userGesture": True})
deadline = time.time() + wait
ws.settimeout(0.5)
while time.time() < deadline:
    try:
        m = json.loads(ws.recv())
    except websocket.WebSocketTimeoutException:
        continue
    if m.get("id") == want:
        print("result:", json.dumps(m.get("result"), indent=1)[:3000])
    elif m.get("method") == "Runtime.consoleAPICalled":
        print("console:", " ".join(str(a.get("value", a.get("description"))) for a in m["params"]["args"]))
    elif m.get("method") == "Runtime.exceptionThrown":
        print("exception:", m["params"]["exceptionDetails"].get("exception", {}).get("description"))
    elif m.get("method") == "Log.entryAdded":
        e = m["params"]["entry"]
        print("log:", e.get("level"), e.get("text"))
ws.close()
