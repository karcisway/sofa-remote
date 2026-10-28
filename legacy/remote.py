"""Sofa Remote: control Windows PCs from a phone browser.

Run:  python remote.py [--port 8765] [--name "Living room"]
Open the printed link on your phone (same Wi-Fi). The phone talks to the PC
over a WebSocket. Layouts come from layouts.json (written when you save from
the phone) or, if that file is missing, from default_layouts.json.
No third-party packages needed; with `qrcode` installed a QR code is printed.
"""

import argparse
import base64
import ctypes
import hashlib
import http.server
import json
import os
import secrets
import socket
import struct
import sys
import threading
import time
import urllib.parse
import webbrowser
from ctypes import wintypes
from http import cookies
from pathlib import Path

if sys.platform != "win32":
    sys.exit("This remote sends Windows input events; run it on Windows.")

HERE = Path(__file__).resolve().parent
WEB_DIR = HERE / "web"
TOKEN_FILE = HERE / ".remote_token"
LAYOUTS_FILE = HERE / "layouts.json"
DEFAULTS_FILE = HERE / "default_layouts.json"
COOKIE_NAME = "remote_token"
MAX_MESSAGE = 2_000_000
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MIME = {".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
        ".css": "text/css; charset=utf-8", ".json": "application/json", ".svg": "image/svg+xml",
        ".png": "image/png", ".ico": "image/x-icon"}

# ---------------------------------------------------------------- Win32 input

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32

INPUT_MOUSE = 0
INPUT_KEYBOARD = 1
KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004
KEYEVENTF_SCANCODE = 0x0008
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_WHEEL = 0x0800
MOUSEEVENTF_HWHEEL = 0x1000
MOUSE_BUTTONS = {"left": (0x0002, 0x0004), "right": (0x0008, 0x0010), "middle": (0x0020, 0x0040)}
CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x0002


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
                ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wintypes.DWORD), ("u", _U)]


kernel32.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
kernel32.GlobalAlloc.restype = ctypes.c_void_p
kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
kernel32.GlobalLock.restype = ctypes.c_void_p
kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
user32.OpenClipboard.argtypes = [wintypes.HWND]
user32.SetClipboardData.argtypes = [wintypes.UINT, ctypes.c_void_p]
user32.SetClipboardData.restype = ctypes.c_void_p
user32.GetClipboardData.argtypes = [wintypes.UINT]
user32.GetClipboardData.restype = ctypes.c_void_p
user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.MapVirtualKeyW.argtypes = [wintypes.UINT, wintypes.UINT]

# Key names usable in layouts -> virtual-key code
KEYS = {
    "playpause": 0xB3, "stop": 0xB2, "next": 0xB0, "prev": 0xB1,
    "volup": 0xAF, "voldown": 0xAE, "mute": 0xAD,
    "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28,
    "home": 0x24, "end": 0x23, "pgup": 0x21, "pgdn": 0x22, "ins": 0x2D, "del": 0x2E,
    "enter": 0x0D, "esc": 0x1B, "tab": 0x09, "space": 0x20, "backspace": 0x08,
    "shift": 0xA0, "rshift": 0xA1, "ctrl": 0xA2, "rctrl": 0xA3, "alt": 0xA4, "altgr": 0xA5,
    "win": 0x5B, "menu": 0x5D, "capslock": 0x14, "printscreen": 0x2C, "pause": 0x13,
    "plus": 0xBB, "minus": 0xBD, "comma": 0xBC, "period": 0xBE, "slash": 0xBF,
    "semicolon": 0xBA, "quote": 0xDE, "bracketleft": 0xDB, "bracketright": 0xDD,
    "backslash": 0xDC, "backquote": 0xC0,
    "nummul": 0x6A, "numplus": 0x6B, "numminus": 0x6D, "numdot": 0x6E, "numdiv": 0x6F,
}
KEYS.update({c: 0x41 + i for i, c in enumerate("abcdefghijklmnopqrstuvwxyz")})
KEYS.update({str(i): 0x30 + i for i in range(10)})
KEYS.update({f"num{i}": 0x60 + i for i in range(10)})
KEYS.update({f"f{i}": 0x6F + i for i in range(1, 25)})
ALIASES = {"return": "enter", "escape": "esc", "control": "ctrl", "meta": "win", "pageup": "pgup",
           "pagedown": "pgdn", "delete": "del", "insert": "ins", "=": "plus", "-": "minus",
           ",": "comma", ".": "period", "/": "slash", ";": "semicolon", "'": "quote",
           "[": "bracketleft", "]": "bracketright", "\\": "backslash", "`": "backquote", " ": "space"}

EXTENDED_VK = {0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E,
               0x5B, 0x5C, 0x5D, 0xA3, 0xA5, 0x6F, 0x2C}
VK_ONLY = set(range(0xA6, 0xB8))  # browser/media/volume keys have no usable scan code

input_lock = threading.Lock()


def vk_of(name):
    if isinstance(name, int):
        return name
    key = str(name).lower()
    vk = KEYS.get(ALIASES.get(key, key))
    if vk is None:
        raise ValueError(f"unknown key: {name}")
    return vk


def _send(*events):
    arr = (INPUT * len(events))(*events)
    user32.SendInput(len(events), arr, ctypes.sizeof(INPUT))


def _key_event(vk, down):
    """Scan-code events where possible: games using DirectInput/raw input ignore VK-only events."""
    flags = 0 if down else KEYEVENTF_KEYUP
    if vk in EXTENDED_VK or vk in VK_ONLY:
        flags |= KEYEVENTF_EXTENDEDKEY
    scan = 0 if vk in VK_ONLY else user32.MapVirtualKeyW(vk, 0)
    if scan:
        flags |= KEYEVENTF_SCANCODE
    ev = INPUT(type=INPUT_KEYBOARD)
    ev.ki = KEYBDINPUT(vk, scan, flags, 0, 0)
    return ev


def key_down(vk):
    _send(_key_event(vk, True))


def key_up(vk):
    _send(_key_event(vk, False))


def press_combo(vks):
    _send(*[_key_event(vk, True) for vk in vks], *[_key_event(vk, False) for vk in reversed(vks)])


def type_text(text, backspaces=0):
    """Type arbitrary Unicode text (any language/emoji)."""
    for _ in range(backspaces):
        press_combo([0x08])
    for ch in text:
        if ch == "\n":
            press_combo([0x0D])
            continue
        if ch == "\t":
            press_combo([0x09])
            continue
        units = ch.encode("utf-16-le")
        events = []
        for i in range(0, len(units), 2):
            code = int.from_bytes(units[i:i + 2], "little")
            for flags in (KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP):
                ev = INPUT(type=INPUT_KEYBOARD)
                ev.ki = KEYBDINPUT(0, code, flags, 0, 0)
                events.append(ev)
        _send(*events)


def _mouse_event(flags, dx=0, dy=0, data=0):
    ev = INPUT(type=INPUT_MOUSE)
    ev.mi = MOUSEINPUT(dx, dy, ctypes.c_uint32(data).value, flags, 0, 0)
    return ev


def mouse_action(kind, dx=0, dy=0):
    if kind == "move":
        _send(_mouse_event(MOUSEEVENTF_MOVE, dx, dy))
    elif kind == "scroll":
        if dy:
            _send(_mouse_event(MOUSEEVENTF_WHEEL, data=dy))
        if dx:
            _send(_mouse_event(MOUSEEVENTF_HWHEEL, data=dx))
    elif kind in ("click", "dblclick", "rclick", "mclick"):
        down, up = MOUSE_BUTTONS[{"rclick": "right", "mclick": "middle"}.get(kind, "left")]
        clicks = 2 if kind == "dblclick" else 1
        _send(*[_mouse_event(f) for _ in range(clicks) for f in (down, up)])
    else:
        raise ValueError(f"unknown mouse action: {kind}")


def mouse_button(button, down):
    flags = MOUSE_BUTTONS[button][0 if down else 1]
    _send(_mouse_event(flags))


# ---------------------------------------------------------------- clipboard / browser

def _open_clipboard():
    for _ in range(10):
        if user32.OpenClipboard(None):
            return True
        time.sleep(0.05)
    return False


def set_clipboard(text):
    data = text.encode("utf-16-le") + b"\0\0"
    if not _open_clipboard():
        raise OSError("clipboard busy")
    try:
        user32.EmptyClipboard()
        h = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(data))
        p = kernel32.GlobalLock(h)
        ctypes.memmove(p, data, len(data))
        kernel32.GlobalUnlock(h)
        user32.SetClipboardData(CF_UNICODETEXT, h)
    finally:
        user32.CloseClipboard()


def get_clipboard():
    if not _open_clipboard():
        raise OSError("clipboard busy")
    try:
        h = user32.GetClipboardData(CF_UNICODETEXT)
        if not h:
            return ""
        p = kernel32.GlobalLock(h)
        try:
            return ctypes.wstring_at(p)
        finally:
            kernel32.GlobalUnlock(h)
    finally:
        user32.CloseClipboard()


def to_url(text):
    """Turn a URL, bare domain or search phrase into an http(s) URL."""
    text = text.strip()
    parsed = urllib.parse.urlparse(text)
    if parsed.scheme in ("http", "https") and parsed.netloc:
        return text
    if text and not any(c.isspace() for c in text) and ("." in text or text.startswith("localhost")):
        return ("http://" if text.startswith("localhost") else "https://") + text
    return "https://www.google.com/search?q=" + urllib.parse.quote_plus(text)


# ---------------------------------------------------------------- layouts config

config_lock = threading.Lock()


def load_config():
    for path in (LAYOUTS_FILE, DEFAULTS_FILE):
        if path.exists():
            return json.loads(path.read_text(encoding="utf-8"))
    return {"layouts": []}


def save_config(config):
    if not isinstance(config, dict) or not isinstance(config.get("layouts"), list):
        raise ValueError("config must be an object with a 'layouts' list")
    tmp = LAYOUTS_FILE.with_suffix(".tmp")
    tmp.write_text(json.dumps(config, indent=2, ensure_ascii=False), encoding="utf-8")
    os.replace(tmp, LAYOUTS_FILE)


def reset_config():
    LAYOUTS_FILE.unlink(missing_ok=True)
    return load_config()


# ---------------------------------------------------------------- WebSocket

clients = set()
clients_lock = threading.Lock()


class WSConn:
    def __init__(self, handler):
        self.sock = handler.connection
        self.rfile = handler.rfile
        self.wlock = threading.Lock()
        self.held_keys = []
        self.held_buttons = set()

    def _read(self, n):
        data = self.rfile.read(n)
        if len(data) < n:
            raise ConnectionError("closed")
        return data

    def send_frame(self, opcode, payload=b""):
        n = len(payload)
        if n < 126:
            header = struct.pack("!BB", 0x80 | opcode, n)
        elif n < 65536:
            header = struct.pack("!BBH", 0x80 | opcode, 126, n)
        else:
            header = struct.pack("!BBQ", 0x80 | opcode, 127, n)
        with self.wlock:
            self.sock.sendall(header + payload)

    def send(self, obj):
        try:
            self.send_frame(0x1, json.dumps(obj, ensure_ascii=False).encode())
        except OSError:
            pass

    def recv(self):
        """Return the next text message, or None when the connection closes."""
        buf = b""
        try:
            while True:
                b1, b2 = self._read(2)
                opcode, n = b1 & 0x0F, b2 & 0x7F
                if n == 126:
                    n = struct.unpack("!H", self._read(2))[0]
                elif n == 127:
                    n = struct.unpack("!Q", self._read(8))[0]
                if n + len(buf) > MAX_MESSAGE:
                    return None
                mask = self._read(4) if b2 & 0x80 else None
                payload = self._read(n)
                if mask and n:
                    full = (mask * (n // 4 + 1))[:n]
                    payload = (int.from_bytes(payload, "big") ^ int.from_bytes(full, "big")).to_bytes(n, "big")
                if opcode == 0x8:
                    self.send_frame(0x8)
                    return None
                if opcode == 0x9:
                    self.send_frame(0xA, payload)
                    continue
                if opcode == 0xA:
                    continue
                buf += payload
                if b1 & 0x80:
                    return buf.decode("utf-8", "replace")
        except (OSError, ConnectionError, ValueError):
            return None

    def release_all(self):
        with input_lock:
            for vk in reversed(self.held_keys):
                key_up(vk)
            for b in self.held_buttons:
                mouse_button(b, False)
        self.held_keys.clear()
        self.held_buttons.clear()


def broadcast(obj, exclude=None):
    with clients_lock:
        targets = [c for c in clients if c is not exclude]
    for c in targets:
        c.send(obj)


def handle_message(conn, m):
    t = m.get("t")
    reply = {"t": "reply", "id": m.get("id")}
    if t == "ping":
        return conn.send({"t": "pong", "id": m.get("id")})
    if t == "key":
        vk = vk_of(m["k"])
        with input_lock:
            press_combo([vk])
    elif t == "combo":
        vks = [vk_of(k) for k in m["keys"]]
        with input_lock:
            press_combo(vks)
    elif t in ("down", "up"):
        vks = [vk_of(k) for k in m["keys"]]
        with input_lock:
            for vk in (vks if t == "down" else reversed(vks)):
                if t == "down" and vk not in conn.held_keys:
                    key_down(vk)
                    conn.held_keys.append(vk)
                elif t == "up" and vk in conn.held_keys:
                    key_up(vk)
                    conn.held_keys.remove(vk)
    elif t == "release":
        conn.release_all()
    elif t == "type":
        with input_lock:
            type_text(str(m.get("text") or ""), max(0, min(1000, int(m.get("backspaces") or 0))))
    elif t == "mouse":
        clamp = lambda v: max(-2000, min(2000, int(v or 0)))
        with input_lock:
            mouse_action(m.get("kind"), clamp(m.get("dx")), clamp(m.get("dy")))
    elif t == "mbtn":
        b = m.get("b", "left")
        if b not in MOUSE_BUTTONS:
            raise ValueError(f"unknown mouse button: {b}")
        down = bool(m.get("down"))
        if down != (b in conn.held_buttons):
            with input_lock:
                mouse_button(b, down)
            (conn.held_buttons.add if down else conn.held_buttons.discard)(b)
    elif t == "clip_get":
        reply["text"] = get_clipboard()
    elif t == "clip_set":
        set_clipboard(str(m.get("text") or ""))
        if m.get("paste"):
            time.sleep(0.05)
            with input_lock:
                press_combo([KEYS["ctrl"], KEYS["v"]])
    elif t == "open":
        reply["url"] = to_url(str(m.get("url") or ""))
        webbrowser.open(reply["url"])
    elif t == "config_get":
        with config_lock:
            reply["config"] = load_config()
    elif t == "config_put":
        with config_lock:
            save_config(m.get("config"))
        broadcast({"t": "config", "config": m["config"]}, exclude=conn)
    elif t == "config_reset":
        with config_lock:
            reply["config"] = reset_config()
        broadcast({"t": "config", "config": reply["config"]}, exclude=conn)
    else:
        raise ValueError(f"unknown message type: {t}")
    if m.get("id") is not None:
        conn.send(reply)


# ---------------------------------------------------------------- HTTP

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    token = ""
    name = ""

    def setup(self):
        super().setup()
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    def log_message(self, fmt, *args):
        pass

    def _authed(self, query=None):
        key = (query or {}).get("k", [""])[0]
        if key and secrets.compare_digest(key, self.token):
            return True
        jar = cookies.SimpleCookie(self.headers.get("Cookie", ""))
        c = jar.get(COOKIE_NAME)
        return c is not None and secrets.compare_digest(c.value, self.token)

    def _send(self, code, body=b"", ctype="text/plain; charset=utf-8", extra=None):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-cache")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        if url.path == "/ws":
            if not self._authed(query):
                return self._send(403, b"forbidden")
            return self._websocket()
        if url.path == "/" and "k" in query:
            if not self._authed(query):
                return self._send(403, b"wrong key")
            cookie = f"{COOKIE_NAME}={self.token}; Path=/; Max-Age=31536000; SameSite=Strict; HttpOnly"
            return self._send(302, extra={"Location": "/", "Set-Cookie": cookie})
        if not self._authed():
            return self._send(403, b"Open the link printed in the PC console (it contains the access key).")
        if url.path == "/api/self":
            body = json.dumps({"name": self.name, "key": self.token}).encode()
            return self._send(200, body, "application/json")
        rel = "index.html" if url.path == "/" else urllib.parse.unquote(url.path).lstrip("/")
        path = (WEB_DIR / rel).resolve()
        if not path.is_relative_to(WEB_DIR) or not path.is_file():
            return self._send(404, b"not found")
        self._send(200, path.read_bytes(), MIME.get(path.suffix, "application/octet-stream"))

    def _websocket(self):
        key = self.headers.get("Sec-WebSocket-Key")
        if not key or self.headers.get("Upgrade", "").lower() != "websocket":
            return self._send(400, b"websocket expected")
        accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.close_connection = True

        conn = WSConn(self)
        with clients_lock:
            clients.add(conn)
        conn.send({"t": "hello", "name": self.name, "keys": sorted(KEYS)})
        try:
            while (raw := conn.recv()) is not None:
                m = None
                try:
                    m = json.loads(raw)
                    handle_message(conn, m)
                except Exception as e:  # report bad messages to the phone, keep the connection
                    msg = {"t": "error", "msg": str(e)}
                    if isinstance(m, dict) and m.get("id") is not None:
                        msg = {"t": "reply", "id": m["id"], "error": str(e)}
                    conn.send(msg)
        finally:
            conn.release_all()
            with clients_lock:
                clients.discard(conn)


def load_token():
    if TOKEN_FILE.exists():
        return TOKEN_FILE.read_text().strip()
    token = secrets.token_urlsafe(9)
    TOKEN_FILE.write_text(token)
    return token


def lan_ip():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("10.255.255.255", 1))  # no packet is sent
        return s.getsockname()[0]
    except OSError:
        return "127.0.0.1"
    finally:
        s.close()


def print_qr(url):
    try:
        import qrcode
    except ImportError:
        print("  (pip install qrcode  to get a scannable QR code here)")
        return
    qr = qrcode.QRCode(border=1)
    qr.add_data(url)
    qr.print_ascii(invert=True)


def main():
    ap = argparse.ArgumentParser(description="Phone remote for this PC")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--name", default=socket.gethostname(), help="name shown on the phone")
    ap.add_argument("--new-key", action="store_true", help="generate a new access key (invalidates old links)")
    args = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)

    if args.new_key and TOKEN_FILE.exists():
        TOKEN_FILE.unlink()
    Handler.token = load_token()
    Handler.name = args.name

    server = http.server.ThreadingHTTPServer(("0.0.0.0", args.port), Handler)
    server.daemon_threads = True
    url = f"http://{lan_ip()}:{args.port}/?k={Handler.token}"
    print(f"\nSofa Remote '{args.name}' running. Open on your phone (same Wi-Fi),")
    print("or paste it under Settings > Add server on a phone already using another PC:\n")
    print(f"  {url}\n")
    print_qr(url)
    source = LAYOUTS_FILE if LAYOUTS_FILE.exists() else DEFAULTS_FILE
    print(f"\nLayouts: {source.name}. Allow Python through Windows Firewall (Private) if asked. Ctrl+C to stop.")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
