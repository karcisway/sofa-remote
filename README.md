# Sofa Remote

Control a PC (Windows, Linux, macOS) from your phone: media keys, volume, touchpad, live
keyboard, shared clipboard, custom layouts, and **real virtual gamepads** (every phone becomes
its own Xbox 360 controller). The PC side is a single executable with the phone web app built in.
Nothing to install on the phone: it runs in the browser, or in the optional Android app.

- [Quick start](#quick-start)
- [Gamepads](#gamepads)
- [Android app](#android-app)
- [Building](#building)
- [Layouts](#layouts)
- [How it works](#how-it-works)

## Quick start

1. Download the binary for your PC from the [Releases](../../releases) page
   (`sofa-remote-windows-x64.exe`, `sofa-remote-linux-x64`, `sofa-remote-macos-arm64`, …).
2. Run it. It prints a link and a QR code.
3. Scan the QR code with the phone (same Wi-Fi), or open the link in its browser.

```
sofa-remote [--port 8765] [--name "Living room"] [--data DIR] [--new-key] [--no-focus]
```

| Option | Does |
| --- | --- |
| `--port` | HTTP port (default 8765) |
| `--name` | Name shown on the phone (default: the computer name) |
| `--data DIR` | Where `layouts.json` and `.remote_token` live (default: next to the executable) |
| `--new-key` | Make a new access key; phones have to scan the new link |
| `--no-focus` | Don't open the phone keyboard by itself when a PC text field gets focus |

**More PCs:** run it on each one, then on the phone use the device menu (top bar) ›
**Add / manage devices** and paste the other PC's link.

**Wrong key:** when the PC made a new key, the phone asks for it. Type the key or paste the
whole link.

### Platform notes

- **Windows**: keys are sent as hardware scan codes, so games using DirectInput or raw input
  see them. A game running as administrator only gets input if sofa-remote also runs as
  administrator.
- **Linux**: needs an X11 session (Xorg). On Wayland only XWayland apps receive keyboard and mouse.
- **macOS**: allow the app under System Settings › Privacy & Security › Accessibility.
- Allow the app through the firewall on private networks (TCP for the page, UDP for WebRTC).

## Gamepads

Each phone gets **its own virtual Xbox 360 controller** on the PC. Four phones on the
**Controller** tab make four separate players in any game that supports controllers (Steam,
emulators, couch co-op games). The controller belongs to the phone: if the phone reconnects
within 30 seconds it gets the same controller back, so the game doesn't drop the player.

| PC | Setup |
| --- | --- |
| Windows | Install the [ViGEmBus driver](https://github.com/nefarius/ViGEmBus/releases) once (the same driver DS4Windows uses) |
| Linux | sofa-remote needs write access to `/dev/uinput` (see below) |
| macOS | Not available. Use the keyboard/mouse **Gamepad** tab instead |

Linux, once:

```
echo 'KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/60-sofa-remote.rules
sudo usermod -aG input $USER       # then log out and back in
sudo modprobe uinput
```

The first time you touch a controller button, the phone shows "Gamepad 2 connected" (the
player number, on Windows) or an error saying what's missing.

There are two built-in game tabs:

- **Controller**: a real gamepad (sticks, ABXY, d-pad, bumpers, triggers, Back/Guide/Start, L3/R3).
- **Gamepad**: keyboard and mouse (WASD stick, mouse-look stick, keys) for games without
  controller support.

You can build your own with the `pad` action and `"mode": "pad"` joysticks (see [Layouts](#layouts)).

## Android app

`sofa-remote-android.apk` (Android 8+, on the Releases page) is the same remote in an app,
plus what a web page can't do:

- **Volume buttons control the PC**, even with the screen off or locked. Pressing both at once
  is play/pause. Under ⚙ › Phone buttons you choose what volume up, down, both, and the
  headset / notification ⏯ ⏭ ⏮ buttons do: a key (`volup`, `space`, `f`), a shortcut
  (`ctrl+right`), `click`/`rclick`/`dblclick`, `scroll:3`, `text:hello` or `open:netflix.com`.
- The keyboard opens by itself when you select a text field on the PC.
- While the app is on screen it keeps the phone's Wi-Fi in low-latency mode (less mouse jitter).

First start: scan the PC's QR code from the app, or paste/share its link to the app. With
"Keep working with the screen locked" on, it shows a media notification. Android sends the
buttons to the app through that notification. If the buttons stop working with the screen
locked, turn on "Silent audio keep-alive".

How it works: a foreground service holds a media session in "remote playback" mode, the way
Chromecast apps take over the volume buttons. It has its own WebSocket to the PC.

## Building

Everything you build ends up in **`release/`**:

```
.\release.ps1                     # Windows: .exe + Linux binary (via Docker) + APK
.\release.ps1 -Skip linux,android # only the .exe
./release.sh                      # Linux/macOS: this machine's binary + APK
```

Or one piece at a time:

```
cargo build --release                         # PC binary in target/release/
cargo build --release --no-default-features   # without WebRTC (smaller, WebSocket only)
cd android && ./gradlew assembleRelease       # APK, copied to release/sofa-remote-android.apk
```

Linux binary from Windows/macOS with Docker:

```
docker run --rm -v "$PWD:/work" -w /work -e CARGO_TARGET_DIR=/work/target-linux rust:1.93-bullseye cargo build --release
```

`SOFA_WEB_DIR=web sofa-remote` serves the phone app from disk instead of the embedded copy, so
you can edit `web/` without rebuilding.

### Android signing

Updates only install over the old app when they're signed with the same key. Make one once and
keep it safe (it's ignored by git):

```
cd android
keytool -genkeypair -keystore sofa-remote.keystore -storepass sofaremote -keypass sofaremote -alias sofa -keyalg RSA -validity 10000 -dname "CN=Sofa Remote"
```

Without it, release builds use the debug key. Different passwords go in
`ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_PASSWORD`.

### GitHub releases

`.github/workflows/release.yml` builds Windows, Linux (x64, ARM64), macOS (ARM, Intel) and the
APK on every push. Push a tag to publish a release with all of them:

```
git tag v0.2.0
git push origin v0.2.0
```

To sign the APK in CI, add the repository secrets `ANDROID_KEYSTORE` (the keystore as base64:
`base64 -w0 sofa-remote.keystore`), `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_PASSWORD`.

### Project files

| Path | What it is |
| --- | --- |
| `src/` | PC server (Rust): HTTP + WebSocket + WebRTC, input injection per OS, virtual gamepads (`gamepad.rs`) |
| `web/` | Phone app (HTML/CSS/JS), embedded into the binary at build time |
| `android/` | Android app (Kotlin): the web app in a WebView, plus volume buttons |
| `default_layouts.json` | Built-in layouts, also embedded |
| `release.ps1`, `release.sh` | Build everything into `release/` |
| `legacy/remote.py` | The old Python server. It no longer matches the current web app |

Next to the executable (or in `--data`) at runtime:

| File | What it is |
| --- | --- |
| `layouts.json` | This PC's own layouts. Created when you save from the phone; overrides the default. Delete it (or JSON › Reset) to go back |
| `.remote_token` | Access key. `--new-key` replaces it |

## Layouts

Edit layouts on the phone (✎), or write the JSON yourself (✎ › JSON, or edit `layouts.json`).

```json
{
  "layouts": [
    {
      "id": "media", "name": "Media",
      "cols": 12, "rowHeight": 64,
      "widgets": [
        {"type": "button", "x": 0, "y": 0, "w": 6, "h": 2, "icon": "⏯", "label": "Play", "style": "accent",
         "action": {"key": "playpause"}}
      ]
    }
  ]
}
```

**Layout fields:** `icon` is the emoji shown on the tab when names don't fit. `allowOverlap: true`
stops widgets pushing each other aside in the editor. `cols` sets the grid width. `rowHeight` is
in px. `fit: true` + `rows` makes a fixed grid that fills the screen (good for gamepads).
`landscape: true` rotates the screen in fullscreen.

**Widget fields:** `type`, `x`, `y`, `w`, `h` (in grid cells), `label`, and `server` (see below).

| type | extra fields |
| --- | --- |
| `button` | `action`, `icon`, `style` (`accent`/`dark`), `color`, `repeat` (`true` or ms) |
| `joystick` | `mode`: `keys` (default; `keys: {"up":"w","left":"a","down":"s","right":"d"}`, `deadzone`), `mouse` (`speed`) or `pad` (analog stick of the virtual gamepad; `stick`: `left`/`right`) |
| `touchpad` | `sens`. Tap: click. Hold still: right click (or the R corner button). Tap then touch again and drag: drag with the left button held (select, move, resize). Double tap: double click |
| `keyboard` | live typing field |
| `clipboard` | shared clipboard + open URL |
| `lockscreen` | phone lock-screen / headset media controls |
| `label` | `text` |

### Actions

| action | does |
| --- | --- |
| `{"key": "f"}` | press a key |
| `{"keys": ["ctrl", "shift", "t"]}` | press a shortcut |
| `{"hold": "w"}` / `{"hold": ["shift", "w"]}` | hold while the finger is down (games) |
| `{"hold_mouse": "left"}` | hold a mouse button (`left`/`right`/`middle`) |
| `{"pad": "a"}` / `{"pad": ["lb", "rb"]}` | hold a virtual gamepad button: `a` `b` `x` `y` `lb` `rb` `lt` `rt` `back` `start` `guide` `ls` `rs` `up` `down` `left` `right` |
| `{"mouse": "click"}` | `click`, `dblclick`, `rclick`, `mclick` |
| `{"scroll": 3}` | scroll down 3 notches (negative = up) |
| `{"move": [dx, dy]}` | move the mouse |
| `{"text": "hello\n"}` | type text |
| `{"open": "netflix.com"}` | open a URL (or search words) on the PC |
| `{"layout": "Gamepad"}` | switch tab on the phone |
| `{"down": "w"}` / `{"up": "w"}` | key down/up only, for macros |
| `{"seq": [ … , {"wait": 200}, … ]}` | macro: steps in order |

Key names: `a`–`z`, `0`–`9`, `f1`–`f24`, `num0`–`num9`, `enter`, `esc`, `tab`, `space`, `backspace`, `del`,
`ins`, `home`, `end`, `pgup`, `pgdn`, `up`/`down`/`left`/`right`, `shift`, `ctrl`, `alt`, `altgr`, `win`,
`capslock`, `printscreen`, `plus`, `minus`, `comma`, `period`, `slash`, `semicolon`, `quote`, `bracketleft`,
`bracketright`, `backslash`, `backquote`, media: `playpause`, `stop`, `next`, `prev`, `volup`, `voldown`, `mute`.

### Targeting other PCs

Add `"server": "<PC name>"` to a widget or an action to send it to another connected PC.
Use `"server": "*"` to send it to all of them, or a list for several:

```json
{"type": "button", "icon": "⏸", "label": "Pause everywhere", "action": {"key": "playpause", "server": "*"}}
```

## How it works

### Latency

- Inputs (keys, mouse, buttons) go as 3–7 byte binary messages. Gamepad updates are 15 bytes
  and carry the whole controller state. Everything else uses JSON.
- Over **WebRTC** (default when available) they use an unordered data channel with no
  retransmissions, which runs over UDP. A lost Wi-Fi packet never holds up the ones behind it
  (with TCP that can stall for 100 ms+). Each key/button event is sent three times (0, 8 and
  40 ms); the server drops duplicates and out-of-date events by sequence number, so keys can't
  get stuck. If WebRTC can't connect, the WebSocket is used.
- Touchpad moves are sent from the touch event itself, without waiting for the next frame.
  They carry the running total of the movement, so a lost packet is made up by the next one
  instead of the cursor stuttering. Acceleration follows a smoothed finger speed.
- Pings: one every 5 s to detect a dead connection, plus one a second while the latency is
  on screen. Keep-awake (Settings) sends a tiny packet only after 80 ms without traffic, and
  only for 3 s after your last touch, so the phone's Wi-Fi doesn't doze off between taps.
- On the PC side a message reaches the OS in microseconds (about 0.1 ms round trip on
  localhost). What's left is Wi-Fi: use 5 GHz, put the PC on Ethernet if you can, and on a
  Wi-Fi PC set the adapter's power saving to maximum performance.

### Notes

- When a phone disconnects or the page is hidden, every held key, mouse button and gamepad
  button is released.
- On Windows the server watches (UI Automation) for a text field getting focus and tells the
  phones, which open their keyboard (⚙ › Open the keyboard…). `--no-focus` turns it off.
- Phone volume buttons can't be read by web pages. The `lockscreen` widget makes the phone show
  media controls on the lock screen instead. The Android app can read them.
