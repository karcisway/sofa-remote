//! Virtual gamepads: every phone gets its own Xbox 360 controller, so games see one real
//! controller per player instead of a shared keyboard.
//!
//! - Windows: through the ViGEmBus driver (https://github.com/nefarius/ViGEmBus/releases).
//! - Linux: through uinput (/dev/uinput must be writable, see the README).
//! - macOS: not available.
//!
//! A pad belongs to the phone (its client id), not to one connection: when the phone reconnects
//! within `KEEP` it gets the same controller back, so games don't drop the player. All devices
//! live on one thread fed by a channel, like the keyboard/mouse input.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// How long a pad stays plugged in after its phone disconnected.
const KEEP: Duration = Duration::from_secs(30);
/// After a failed plug-in, wait this long before trying again.
const RETRY: Duration = Duration::from_secs(5);

/// Controller state in XInput layout: `buttons` uses the XInput bits (A = 0x1000, …), sticks are
/// -32768..32767 with positive y = up, triggers 0..255.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PadState {
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

impl PadState {
    /// Wire format (12 bytes): buttons u16, lt u8, rt u8, lx ly rx ry i16, all little-endian.
    pub fn parse(p: &[u8]) -> Option<Self> {
        let i16_at = |i: usize| Some(i16::from_le_bytes([*p.get(i)?, *p.get(i + 1)?]));
        Some(PadState {
            buttons: u16::from_le_bytes([*p.first()?, *p.get(1)?]),
            lt: *p.get(2)?,
            rt: *p.get(3)?,
            lx: i16_at(4)?,
            ly: i16_at(6)?,
            rx: i16_at(8)?,
            ry: i16_at(10)?,
        })
    }
}

trait VirtualPad: Send {
    fn update(&mut self, state: &PadState) -> Result<(), String>;
    /// Player number shown on the phone, when the OS assigns one.
    fn player(&mut self) -> Option<u32>;
}

enum Msg {
    /// A connection started using this phone's pad.
    Attach(String),
    /// A connection that used it closed.
    Detach(String),
    Update(String, PadState),
    /// Plug the pad in now and report the player number or why it failed.
    Open(String, Sender<Result<Option<u32>, String>>),
}

#[derive(Default)]
struct Entry {
    pad: Option<Box<dyn VirtualPad>>,
    users: u32,
    idle_since: Option<Instant>,
    retry_at: Option<Instant>,
    state: PadState,
}

impl Entry {
    fn ensure(&mut self) -> Result<&mut Box<dyn VirtualPad>, String> {
        if self.pad.is_none() {
            if self.retry_at.is_some_and(|t| Instant::now() < t) {
                return Err("virtual gamepad unavailable".into());
            }
            match platform::create() {
                Ok(p) => self.pad = Some(p),
                Err(e) => {
                    self.retry_at = Some(Instant::now() + RETRY);
                    return Err(e);
                }
            }
        }
        Ok(self.pad.as_mut().unwrap())
    }

    fn set(&mut self, state: PadState) {
        self.state = state;
        if let Ok(pad) = self.ensure()
            && pad.update(&state).is_err()
        {
            self.pad = None; // unplugged from outside: plug a new one in next time
        }
    }
}

fn run(rx: Receiver<Msg>) {
    let mut pads: HashMap<String, Entry> = HashMap::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Msg::Attach(c)) => {
                let e = pads.entry(c).or_default();
                e.users += 1;
                e.idle_since = None;
            }
            Ok(Msg::Detach(c)) => {
                if let Some(e) = pads.get_mut(&c) {
                    e.users = e.users.saturating_sub(1);
                    if e.users == 0 {
                        e.idle_since = Some(Instant::now());
                        if e.state != PadState::default() {
                            e.set(PadState::default());
                        }
                    }
                }
            }
            Ok(Msg::Update(c, state)) => pads.entry(c).or_default().set(state),
            Ok(Msg::Open(c, reply)) => {
                let e = pads.entry(c).or_insert_with(|| Entry { idle_since: Some(Instant::now()), ..Default::default() });
                e.retry_at = None;
                let state = e.state;
                let r = e.ensure().and_then(|pad| {
                    pad.update(&state)?;
                    Ok(pad.player())
                });
                let _ = reply.send(r);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        // dropping an entry unplugs its controller
        pads.retain(|_, e| e.idle_since.is_none_or(|t| t.elapsed() < KEEP));
    }
}

fn sender() -> &'static Sender<Msg> {
    static TX: OnceLock<Sender<Msg>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("gamepads".into()).spawn(move || run(rx)).expect("gamepad thread");
        tx
    })
}

pub fn attach(client: &str) {
    let _ = sender().send(Msg::Attach(client.to_string()));
}

pub fn detach(client: &str) {
    let _ = sender().send(Msg::Detach(client.to_string()));
}

pub fn update(client: &str, state: PadState) {
    let _ = sender().send(Msg::Update(client.to_string(), state));
}

/// Plug in this phone's pad (blocking). Ok(Some(n)) = player n.
pub fn open(client: &str) -> Result<Option<u32>, String> {
    let (tx, rx) = channel();
    sender().send(Msg::Open(client.to_string(), tx)).map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "gamepad timeout".to_string())?
}

// ------------------------------------------------------------------ Windows: ViGEmBus

#[cfg(windows)]
mod platform {
    use super::{PadState, VirtualPad};
    use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

    struct Pad(Xbox360Wired<Client>);

    pub fn create() -> Result<Box<dyn VirtualPad>, String> {
        let client = Client::connect().map_err(|_| {
            "Virtual gamepads need the ViGEmBus driver: install it from github.com/nefarius/ViGEmBus/releases".to_string()
        })?;
        let mut pad = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        pad.plugin().map_err(|e| format!("gamepad plug-in failed: {e}"))?;
        pad.wait_ready().map_err(|e| format!("gamepad not ready: {e}"))?;
        Ok(Box::new(Pad(pad)))
    }

    impl VirtualPad for Pad {
        fn update(&mut self, s: &PadState) -> Result<(), String> {
            let g = XGamepad {
                buttons: XButtons(s.buttons),
                left_trigger: s.lt,
                right_trigger: s.rt,
                thumb_lx: s.lx,
                thumb_ly: s.ly,
                thumb_rx: s.rx,
                thumb_ry: s.ry,
            };
            self.0.update(&g).map_err(|e| e.to_string())
        }

        fn player(&mut self) -> Option<u32> {
            self.0.get_user_index().ok().map(|i| i + 1)
        }
    }
}

// ------------------------------------------------------------------ Linux: uinput

#[cfg(target_os = "linux")]
mod platform {
    use super::{PadState, VirtualPad};
    use evdev::uinput::VirtualDevice;
    use evdev::{AbsInfo, AbsoluteAxisCode as Abs, AbsoluteAxisEvent, AttributeSet, BusType, InputEvent, InputId,
                KeyCode, KeyEvent, UinputAbsSetup};

    /// XInput button bit -> Linux code, the way the xpad driver reports an Xbox 360 pad
    /// (the d-pad goes as the hat axes below).
    const BUTTONS: &[(u16, KeyCode)] = &[
        (0x1000, KeyCode::BTN_SOUTH), (0x2000, KeyCode::BTN_EAST), (0x4000, KeyCode::BTN_NORTH),
        (0x8000, KeyCode::BTN_WEST), (0x0100, KeyCode::BTN_TL), (0x0200, KeyCode::BTN_TR),
        (0x0020, KeyCode::BTN_SELECT), (0x0010, KeyCode::BTN_START), (0x0400, KeyCode::BTN_MODE),
        (0x0040, KeyCode::BTN_THUMBL), (0x0080, KeyCode::BTN_THUMBR),
    ];

    struct Pad(VirtualDevice);

    pub fn create() -> Result<Box<dyn VirtualPad>, String> {
        let build = || -> std::io::Result<VirtualDevice> {
            let keys: AttributeSet<KeyCode> = BUTTONS.iter().map(|(_, k)| *k).collect();
            let stick = AbsInfo::new(0, -32768, 32767, 16, 128, 0);
            let trigger = AbsInfo::new(0, 0, 255, 0, 0, 0);
            let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
            let mut b = VirtualDevice::builder()?
                .name("Sofa Remote Gamepad")
                // Microsoft X-Box 360 pad: games and SDL map it without configuration
                .input_id(InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0110))
                .with_keys(&keys)?;
            for (axis, info) in [(Abs::ABS_X, stick), (Abs::ABS_Y, stick), (Abs::ABS_RX, stick), (Abs::ABS_RY, stick),
                                 (Abs::ABS_Z, trigger), (Abs::ABS_RZ, trigger), (Abs::ABS_HAT0X, hat), (Abs::ABS_HAT0Y, hat)] {
                b = b.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
            }
            b.build()
        };
        build().map(|d| Box::new(Pad(d)) as Box<dyn VirtualPad>).map_err(|e| {
            format!("Virtual gamepads need write access to /dev/uinput ({e}); see the README")
        })
    }

    impl VirtualPad for Pad {
        fn update(&mut self, s: &PadState) -> Result<(), String> {
            let abs = |a, v: i32| *AbsoluteAxisEvent::new(a, v);
            let flip = |v: i16| -(v as i32).max(-32767); // XInput y is up, evdev y is down
            let hat = |neg: u16, pos: u16| (s.buttons & pos != 0) as i32 - (s.buttons & neg != 0) as i32;
            let mut events: Vec<InputEvent> = BUTTONS.iter()
                .map(|(bit, k)| *KeyEvent::new(*k, (s.buttons & bit != 0) as i32))
                .collect();
            events.extend([
                abs(Abs::ABS_X, s.lx as i32), abs(Abs::ABS_Y, flip(s.ly)),
                abs(Abs::ABS_RX, s.rx as i32), abs(Abs::ABS_RY, flip(s.ry)),
                abs(Abs::ABS_Z, s.lt as i32), abs(Abs::ABS_RZ, s.rt as i32),
                abs(Abs::ABS_HAT0X, hat(0x0004, 0x0008)), abs(Abs::ABS_HAT0Y, hat(0x0001, 0x0002)),
            ]);
            self.0.emit(&events).map_err(|e| e.to_string())
        }

        fn player(&mut self) -> Option<u32> {
            None
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::VirtualPad;

    pub fn create() -> Result<Box<dyn VirtualPad>, String> {
        Err("Virtual gamepads work on Windows and Linux only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wire_state() {
        let mut p = vec![];
        p.extend(0x1001u16.to_le_bytes());
        p.extend([10, 255]);
        for v in [-32768i16, 32767, 0, -1] {
            p.extend(v.to_le_bytes());
        }
        let s = PadState::parse(&p).unwrap();
        assert_eq!(s, PadState { buttons: 0x1001, lt: 10, rt: 255, lx: -32768, ly: 32767, rx: 0, ry: -1 });
        assert!(PadState::parse(&p[..11]).is_none());
    }
}
