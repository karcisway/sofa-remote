//! Windows backend: SendInput with hardware scan codes, so games that read DirectInput /
//! raw input (and ignore virtual-key-only events) see the keys too.

use super::{Button, Platform};
use crate::keys::is_media;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

const EXTENDED: &[u16] = &[0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2C, 0x2D, 0x2E,
                           0x5B, 0x5C, 0x5D, 0x6F, 0xA3, 0xA5];

pub struct Backend;

impl Backend {
    pub fn new() -> Result<Self, String> {
        Ok(Backend)
    }
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}

fn keyboard(vk: u16, scan: u16, flags: u32) -> INPUT {
    let mut input = INPUT { r#type: INPUT_KEYBOARD, ..Default::default() };
    input.Anonymous.ki = KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    input
}

fn mouse(flags: u32, dx: i32, dy: i32, data: i32) -> INPUT {
    let mut input = INPUT { r#type: INPUT_MOUSE, ..Default::default() };
    input.Anonymous.mi = MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    input
}

impl Platform for Backend {
    fn key(&mut self, vk: u16, down: bool) {
        let mut flags = if down { 0 } else { KEYEVENTF_KEYUP };
        if EXTENDED.contains(&vk) || is_media(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        // Media/volume keys have no usable scan code: send them as virtual keys
        let scan = if is_media(vk) { 0 } else { unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 } };
        if scan != 0 {
            flags |= KEYEVENTF_SCANCODE;
        }
        send(&[keyboard(vk, scan, flags)]);
    }

    fn text(&mut self, text: &str) {
        let mut inputs = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            inputs.push(keyboard(0, unit, KEYEVENTF_UNICODE));
            inputs.push(keyboard(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        send(&inputs);
    }

    fn move_rel(&mut self, dx: i32, dy: i32) {
        send(&[mouse(MOUSEEVENTF_MOVE, dx, dy, 0)]);
    }

    fn button(&mut self, button: Button, down: bool) {
        let flags = match (button, down) {
            (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (Button::Left, false) => MOUSEEVENTF_LEFTUP,
            (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
            (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        send(&[mouse(flags, 0, 0, 0)]);
    }

    fn scroll(&mut self, dy: i32, dx: i32) {
        if dy != 0 {
            send(&[mouse(MOUSEEVENTF_WHEEL, 0, 0, dy)]);
        }
        if dx != 0 {
            send(&[mouse(MOUSEEVENTF_HWHEEL, 0, 0, dx)]);
        }
    }
}
