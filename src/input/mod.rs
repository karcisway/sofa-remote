//! Input injection. All OS input happens on one dedicated thread fed by a channel, so callers
//! never block and events reach the OS in the order they arrived.

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Backend;

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other::Backend;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    pub fn from_index(i: u8) -> Option<Self> {
        [Self::Left, Self::Right, Self::Middle].get(i as usize).copied()
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Left,
    Double,
    Right,
    Middle,
}

#[derive(Debug)]
pub enum Cmd {
    Key { vk: u16, down: bool },
    /// Press all keys in order, release in reverse.
    Combo(Vec<u16>),
    Text { text: String, backspaces: u32 },
    Move { dx: i32, dy: i32 },
    Button { button: Button, down: bool },
    Click(Click),
    /// Wheel units: 120 per notch, positive = up/right (Windows convention).
    Scroll { dy: i32, dx: i32 },
}

/// The interface each platform backend implements.
pub trait Platform {
    fn key(&mut self, vk: u16, down: bool);
    fn text(&mut self, text: &str);
    fn move_rel(&mut self, dx: i32, dy: i32);
    fn button(&mut self, button: Button, down: bool);
    fn scroll(&mut self, dy: i32, dx: i32);
}

fn run(backend: &mut impl Platform, cmd: Cmd) {
    match cmd {
        Cmd::Key { vk, down } => backend.key(vk, down),
        Cmd::Combo(vks) => {
            vks.iter().for_each(|&vk| backend.key(vk, true));
            vks.iter().rev().for_each(|&vk| backend.key(vk, false));
        }
        Cmd::Text { text, backspaces } => {
            for _ in 0..backspaces {
                backend.key(crate::keys::VK_BACK, true);
                backend.key(crate::keys::VK_BACK, false);
            }
            // Enter and Tab as real keys so they work in every app
            let mut buf = String::new();
            for ch in text.chars() {
                let vk = match ch {
                    '\n' => crate::keys::VK_RETURN,
                    '\t' => crate::keys::VK_TAB,
                    _ => {
                        buf.push(ch);
                        continue;
                    }
                };
                if !buf.is_empty() {
                    backend.text(&std::mem::take(&mut buf));
                }
                backend.key(vk, true);
                backend.key(vk, false);
            }
            if !buf.is_empty() {
                backend.text(&buf);
            }
        }
        Cmd::Move { dx, dy } => backend.move_rel(dx, dy),
        Cmd::Button { button, down } => backend.button(button, down),
        Cmd::Click(kind) => {
            let (button, times) = match kind {
                Click::Left => (Button::Left, 1),
                Click::Double => (Button::Left, 2),
                Click::Right => (Button::Right, 1),
                Click::Middle => (Button::Middle, 1),
            };
            for _ in 0..times {
                backend.button(button, true);
                backend.button(button, false);
            }
        }
        Cmd::Scroll { dy, dx } => backend.scroll(dy, dx),
    }
}

static SENDER: OnceLock<Sender<Cmd>> = OnceLock::new();

/// Start the input thread. Returns an error message if the OS refuses input injection.
pub fn start() -> Result<(), String> {
    let (tx, rx) = channel::<Cmd>();
    let (ready_tx, ready_rx) = channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("input".into())
        .spawn(move || {
            let mut backend = match Backend::new() {
                Ok(b) => {
                    let _ = ready_tx.send(Ok(()));
                    b
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            for cmd in rx {
                run(&mut backend, cmd);
            }
        })
        .map_err(|e| e.to_string())?;
    ready_rx.recv().map_err(|e| e.to_string())??;
    SENDER.set(tx).map_err(|_| "input already started".to_string())
}

pub fn send(cmd: Cmd) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(cmd);
    }
}
