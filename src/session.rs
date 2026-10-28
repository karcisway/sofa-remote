//! Per-phone state and the binary fast-path protocol.
//!
//! Binary message: `[type u8][seq u16 LE][payload]`. The same bytes arrive over the WebSocket
//! (reliable) or a WebRTC data channel (unordered, no retransmits). Over WebRTC the phone sends
//! each key/button event several times; `seq` lets us drop the duplicates and ignore an event
//! that arrives after a newer one for the same key, so keys can never get stuck down.
//!
//! Mouse moves go as MOVE_TOTAL: the running total of the movement since the connection opened.
//! The server moves by the difference from the last total it saw, so a lost packet's movement
//! arrives with the next one and a late one is ignored, instead of the cursor skipping or jumping.
//!
//! PAD carries the whole state of the phone's virtual gamepad (see gamepad.rs), so a lost or late
//! packet is simply replaced by the next one.

use crate::gamepad::{self, PadState};
use crate::input::{self, Button, Click, Cmd};
use std::collections::{HashMap, VecDeque};

pub const T_MOVE: u8 = 0x01;
pub const T_KEY_DOWN: u8 = 0x02;
pub const T_KEY_UP: u8 = 0x03;
pub const T_COMBO: u8 = 0x04;
pub const T_BTN_DOWN: u8 = 0x05;
pub const T_BTN_UP: u8 = 0x06;
pub const T_CLICK: u8 = 0x07;
pub const T_SCROLL: u8 = 0x08;
pub const T_PING: u8 = 0x09;
pub const T_RELEASE: u8 = 0x0A;
pub const T_MOVE_TOTAL: u8 = 0x0B;
pub const T_PAD: u8 = 0x0C;

const MOVE_ID: u32 = 0x2_0000;
const PAD_ID: u32 = 0x3_0000;

#[derive(Default)]
pub struct Session {
    held_keys: Vec<u16>,
    held_buttons: Vec<Button>,
    recent: VecDeque<u16>,
    last_seq: HashMap<u32, u16>,
    move_total: (i32, i32),
    /// Identifies the phone across reconnects; its virtual gamepad is keyed by it.
    client: String,
    pad: Option<PadState>,
}

fn i16_at(p: &[u8], i: usize) -> Option<i32> {
    Some(i16::from_le_bytes([*p.get(i)?, *p.get(i + 1)?]) as i32)
}
fn i32_at(p: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_le_bytes(p.get(i..i + 4)?.try_into().ok()?))
}
fn u16_at(p: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*p.get(i)?, *p.get(i + 1)?]))
}

impl Session {
    pub fn new(client: String) -> Self {
        let mut s = Session::default();
        s.client = client;
        s
    }

    pub fn client(&self) -> &str {
        &self.client
    }

    fn pad(&mut self, state: PadState) {
        if self.pad.is_none() {
            gamepad::attach(&self.client);
        }
        if self.pad != Some(state) {
            gamepad::update(&self.client, state);
        }
        self.pad = Some(state);
    }

    pub fn key(&mut self, vk: u16, down: bool) {
        let held = self.held_keys.iter().position(|&k| k == vk);
        match (down, held) {
            (true, None) => {
                self.held_keys.push(vk);
                input::send(Cmd::Key { vk, down: true });
            }
            (false, Some(i)) => {
                self.held_keys.remove(i);
                input::send(Cmd::Key { vk, down: false });
            }
            _ => {}
        }
    }

    pub fn button(&mut self, button: Button, down: bool) {
        let held = self.held_buttons.iter().position(|&b| b == button);
        match (down, held) {
            (true, None) => self.held_buttons.push(button),
            (false, Some(i)) => {
                self.held_buttons.remove(i);
            }
            _ => return,
        }
        input::send(Cmd::Button { button, down });
    }

    pub fn release_all(&mut self) {
        for vk in std::mem::take(&mut self.held_keys).into_iter().rev() {
            input::send(Cmd::Key { vk, down: false });
        }
        for button in std::mem::take(&mut self.held_buttons) {
            input::send(Cmd::Button { button, down: false });
        }
        if self.pad.is_some() {
            self.pad(PadState::default());
        }
    }

    /// First time we see this sequence number? (seq 0 = not tracked)
    fn fresh(&mut self, seq: u16) -> bool {
        if seq == 0 {
            return true;
        }
        if self.recent.contains(&seq) {
            return false;
        }
        if self.recent.len() >= 512 {
            self.recent.pop_front();
        }
        self.recent.push_back(seq);
        true
    }

    /// Is this the newest event so far for `id` (a key or button)?
    fn newest(&mut self, id: u32, seq: u16) -> bool {
        if seq == 0 {
            return true;
        }
        if let Some(&last) = self.last_seq.get(&id) {
            let d = seq.wrapping_sub(last);
            if d == 0 || d >= 0x8000 {
                return false;
            }
        }
        self.last_seq.insert(id, seq);
        true
    }

    /// Handle one binary message; returns bytes to send back (pong) if any.
    pub fn binary(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (&kind, rest) = data.split_first()?;
        let seq = u16_at(rest, 0)?;
        let p = &rest[2..];
        if kind == T_PING {
            return Some(data.to_vec());
        }
        if !self.fresh(seq) {
            return None;
        }
        match kind {
            T_MOVE => input::send(Cmd::Move { dx: i16_at(p, 0)?, dy: i16_at(p, 2)? }),
            T_MOVE_TOTAL => {
                let (x, y) = (i32_at(p, 0)?, i32_at(p, 4)?);
                if self.newest(MOVE_ID, seq) {
                    let (dx, dy) = (x.wrapping_sub(self.move_total.0), y.wrapping_sub(self.move_total.1));
                    self.move_total = (x, y);
                    if dx != 0 || dy != 0 {
                        input::send(Cmd::Move { dx, dy });
                    }
                }
            }
            T_KEY_DOWN | T_KEY_UP => {
                let vk = u16_at(p, 0)?;
                if self.newest(vk as u32, seq) {
                    self.key(vk, kind == T_KEY_DOWN);
                }
            }
            T_COMBO => {
                let n = *p.first()? as usize;
                let vks = (0..n).map(|i| u16_at(p, 1 + i * 2)).collect::<Option<Vec<_>>>()?;
                input::send(Cmd::Combo(vks));
            }
            T_BTN_DOWN | T_BTN_UP => {
                let b = *p.first()?;
                let button = Button::from_index(b)?;
                if self.newest(0x1_0000 | b as u32, seq) {
                    self.button(button, kind == T_BTN_DOWN);
                }
            }
            T_CLICK => {
                let click = [Click::Left, Click::Double, Click::Right, Click::Middle].get(*p.first()? as usize)?;
                input::send(Cmd::Click(*click));
            }
            T_SCROLL => input::send(Cmd::Scroll { dy: i16_at(p, 0)?, dx: i16_at(p, 2).unwrap_or(0) }),
            T_RELEASE => self.release_all(),
            T_PAD => {
                let state = PadState::parse(p)?;
                if self.newest(PAD_ID, seq) {
                    self.pad(state);
                }
            }
            _ => {}
        }
        None
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.pad.is_some() {
            gamepad::detach(&self.client);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_msg(kind: u8, seq: u16, vk: u16) -> Vec<u8> {
        let mut m = vec![kind];
        m.extend(seq.to_le_bytes());
        m.extend(vk.to_le_bytes());
        m
    }

    #[test]
    fn duplicates_and_late_packets_never_leave_a_key_down() {
        let mut s = Session::default();
        let down = key_msg(T_KEY_DOWN, 10, 0x57);
        let up = key_msg(T_KEY_UP, 11, 0x57);
        s.binary(&down);
        assert_eq!(s.held_keys, vec![0x57]);
        s.binary(&down); // duplicate
        assert_eq!(s.held_keys, vec![0x57]);
        s.binary(&up);
        assert!(s.held_keys.is_empty());
        s.binary(&down); // late duplicate of the older press
        assert!(s.held_keys.is_empty());
    }

    #[test]
    fn up_arriving_before_down_wins() {
        let mut s = Session::default();
        s.binary(&key_msg(T_KEY_UP, 21, 0x41));
        s.binary(&key_msg(T_KEY_DOWN, 20, 0x41));
        assert!(s.held_keys.is_empty());
    }

    #[test]
    fn sequence_wraps_around() {
        let mut s = Session::default();
        s.binary(&key_msg(T_KEY_DOWN, 65535, 0x41));
        s.binary(&key_msg(T_KEY_UP, 1, 0x41));
        assert!(s.held_keys.is_empty());
    }

    #[test]
    fn move_totals_survive_loss_and_reordering() {
        let mut s = Session::default();
        let mv = |seq: u16, x: i32, y: i32| {
            let mut m = vec![T_MOVE_TOTAL];
            m.extend(seq.to_le_bytes());
            m.extend(x.to_le_bytes());
            m.extend(y.to_le_bytes());
            m
        };
        s.binary(&mv(1, 5, -2));
        assert_eq!(s.move_total, (5, -2));
        // seq 2 lost; seq 3 carries its movement too
        s.binary(&mv(3, 20, 4));
        assert_eq!(s.move_total, (20, 4));
        s.binary(&mv(2, 12, 1)); // late: ignored
        assert_eq!(s.move_total, (20, 4));
    }

    #[test]
    fn ping_is_echoed() {
        let mut s = Session::default();
        let ping = [T_PING, 0, 0, 7, 0];
        assert_eq!(s.binary(&ping), Some(ping.to_vec()));
    }
}
