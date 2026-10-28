# Changes

## 0.1.9 — 2026-10-28

- Keep virtual controllers connected through temporary XInput queue delays. Retry failed phone
  connections, resend their latest state after connecting, and discard stale controllers.
- Report missing, inaccessible, and incompatible ViGEmBus drivers separately. Reject missing
  player-number data instead of reporting player one for every controller.
- Bundle an Android QR camera scanner and decoder. Scan from setup, Settings, the device menu,
  or the native offline screen without downloading a Google Play scanner component.
- Confirm disconnect when tapping the offline status. Disconnect stops reconnect attempts,
  clears the Android connection, stops background hardware-button control, and returns to setup.
  Cancel keeps reconnecting.
- Match the PC and Android version names at 0.1.9; increase Android's version code to 10.
  Show the PC version at startup and with `--version`.

Update both the PC server and Android app to expose the new scanner and offline controls.
The phone UI is embedded in the PC server and loaded by the Android WebView.

Build commands, artifact checksums, verification results, and remaining validation limits are
recorded in [BUILD-INFO.md](BUILD-INFO.md).
