//! Linux (X11) and macOS backend built on enigo. Virtual-key codes from the wire are
//! translated to enigo keys; printable keys are sent as their character.

use super::{Button, Platform};
use enigo::{Axis, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

pub struct Backend {
    enigo: Enigo,
    /// Wheel units not yet worth a whole notch.
    scroll_rest: (i32, i32),
}

impl Backend {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default()).map_err(|e| {
            if cfg!(target_os = "macos") {
                format!("{e}. Allow this app under System Settings > Privacy & Security > Accessibility.")
            } else {
                format!("{e}. An X11 session is required (on Wayland, log in with Xorg).")
            }
        })?;
        Ok(Backend { enigo, scroll_rest: (0, 0) })
    }
}

fn to_key(vk: u16) -> Option<Key> {
    Some(match vk {
        0x41..=0x5A => Key::Unicode((b'a' + (vk - 0x41) as u8) as char),
        0x30..=0x39 => Key::Unicode((b'0' + (vk - 0x30) as u8) as char),
        0x60..=0x69 => [Key::Numpad0, Key::Numpad1, Key::Numpad2, Key::Numpad3, Key::Numpad4,
                        Key::Numpad5, Key::Numpad6, Key::Numpad7, Key::Numpad8, Key::Numpad9][(vk - 0x60) as usize],
        0x70..=0x83 => [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10,
                        Key::F11, Key::F12, Key::F13, Key::F14, Key::F15, Key::F16, Key::F17, Key::F18, Key::F19,
                        Key::F20][(vk - 0x70) as usize],
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0D => Key::Return,
        0x14 => Key::CapsLock,
        0x1B => Key::Escape,
        0x20 => Key::Space,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::LeftArrow,
        0x26 => Key::UpArrow,
        0x27 => Key::RightArrow,
        0x28 => Key::DownArrow,
        0x2E => Key::Delete,
        0x5B | 0x5C => Key::Meta,
        0xA0 => Key::LShift,
        0xA1 => Key::RShift,
        0xA2 => Key::LControl,
        0xA3 => Key::RControl,
        0xA4 | 0xA5 => Key::Alt,
        0xAD => Key::VolumeMute,
        0xAE => Key::VolumeDown,
        0xAF => Key::VolumeUp,
        0xB0 => Key::MediaNextTrack,
        0xB1 => Key::MediaPrevTrack,
        0xB3 => Key::MediaPlayPause,
        0xBA => Key::Unicode(';'),
        0xBB => Key::Unicode('='),
        0xBC => Key::Unicode(','),
        0xBD => Key::Unicode('-'),
        0xBE => Key::Unicode('.'),
        0xBF => Key::Unicode('/'),
        0xC0 => Key::Unicode('`'),
        0xDB => Key::Unicode('['),
        0xDC => Key::Unicode('\\'),
        0xDD => Key::Unicode(']'),
        0xDE => Key::Unicode('\''),
        0x6A => Key::Unicode('*'),
        0x6B => Key::Unicode('+'),
        0x6D => Key::Unicode('-'),
        0x6E => Key::Unicode('.'),
        0x6F => Key::Unicode('/'),
        #[cfg(not(target_os = "macos"))]
        0x2D => Key::Insert,
        #[cfg(not(target_os = "macos"))]
        0x2C => Key::PrintScr,
        #[cfg(not(target_os = "macos"))]
        0x13 => Key::Pause,
        #[cfg(not(target_os = "macos"))]
        0xB2 => Key::MediaStop,
        _ => return None,
    })
}

fn notches(rest: &mut i32, delta: i32) -> i32 {
    *rest += delta;
    let n = *rest / 120;
    *rest -= n * 120;
    n
}

impl Platform for Backend {
    fn key(&mut self, vk: u16, down: bool) {
        if let Some(key) = to_key(vk) {
            let dir = if down { Direction::Press } else { Direction::Release };
            let _ = self.enigo.key(key, dir);
        }
    }

    fn text(&mut self, text: &str) {
        let _ = self.enigo.text(text);
    }

    fn move_rel(&mut self, dx: i32, dy: i32) {
        let _ = self.enigo.move_mouse(dx, dy, Coordinate::Rel);
    }

    fn button(&mut self, button: Button, down: bool) {
        let b = match button {
            Button::Left => enigo::Button::Left,
            Button::Right => enigo::Button::Right,
            Button::Middle => enigo::Button::Middle,
        };
        let _ = self.enigo.button(b, if down { Direction::Press } else { Direction::Release });
    }

    fn scroll(&mut self, dy: i32, dx: i32) {
        // enigo: positive = down/right; wire format: positive = up/right
        let v = notches(&mut self.scroll_rest.0, dy);
        if v != 0 {
            let _ = self.enigo.scroll(-v, Axis::Vertical);
        }
        let h = notches(&mut self.scroll_rest.1, dx);
        if h != 0 {
            let _ = self.enigo.scroll(h, Axis::Horizontal);
        }
    }
}
