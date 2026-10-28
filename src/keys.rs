//! Key names used in layouts. Every key is identified on the wire by its Windows virtual-key
//! code; non-Windows backends translate those codes to their own key types.

use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_V: u16 = 0x56;

const NAMED: &[(&str, u16)] = &[
    ("playpause", 0xB3), ("stop", 0xB2), ("next", 0xB0), ("prev", 0xB1),
    ("volup", 0xAF), ("voldown", 0xAE), ("mute", 0xAD),
    ("left", 0x25), ("up", 0x26), ("right", 0x27), ("down", 0x28),
    ("home", 0x24), ("end", 0x23), ("pgup", 0x21), ("pgdn", 0x22), ("ins", 0x2D), ("del", 0x2E),
    ("enter", 0x0D), ("esc", 0x1B), ("tab", 0x09), ("space", 0x20), ("backspace", 0x08),
    ("shift", 0xA0), ("rshift", 0xA1), ("ctrl", 0xA2), ("rctrl", 0xA3), ("alt", 0xA4), ("altgr", 0xA5),
    ("win", 0x5B), ("menu", 0x5D), ("capslock", 0x14), ("printscreen", 0x2C), ("pause", 0x13),
    ("plus", 0xBB), ("minus", 0xBD), ("comma", 0xBC), ("period", 0xBE), ("slash", 0xBF),
    ("semicolon", 0xBA), ("quote", 0xDE), ("bracketleft", 0xDB), ("bracketright", 0xDD),
    ("backslash", 0xDC), ("backquote", 0xC0),
    ("nummul", 0x6A), ("numplus", 0x6B), ("numminus", 0x6D), ("numdot", 0x6E), ("numdiv", 0x6F),
];

const ALIASES: &[(&str, &str)] = &[
    ("return", "enter"), ("escape", "esc"), ("control", "ctrl"), ("meta", "win"), ("cmd", "win"),
    ("pageup", "pgup"), ("pagedown", "pgdn"), ("delete", "del"), ("insert", "ins"),
    ("=", "plus"), ("-", "minus"), (",", "comma"), (".", "period"), ("/", "slash"), (";", "semicolon"),
    ("'", "quote"), ("[", "bracketleft"), ("]", "bracketright"), ("\\", "backslash"), ("`", "backquote"),
    (" ", "space"),
];

/// name -> virtual-key code, for every key name layouts may use.
pub fn table() -> &'static BTreeMap<String, u16> {
    static TABLE: OnceLock<BTreeMap<String, u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t: BTreeMap<String, u16> = NAMED.iter().map(|(n, v)| (n.to_string(), *v)).collect();
        for (i, c) in ('a'..='z').enumerate() {
            t.insert(c.to_string(), 0x41 + i as u16);
        }
        for i in 0..10u16 {
            t.insert(i.to_string(), 0x30 + i);
            t.insert(format!("num{i}"), 0x60 + i);
        }
        for i in 1..=24u16 {
            t.insert(format!("f{i}"), 0x6F + i);
        }
        t
    })
}

pub fn code_of(name: &str) -> Option<u16> {
    let lower = name.to_lowercase();
    let key = ALIASES.iter().find(|(a, _)| *a == lower).map(|(_, n)| *n).unwrap_or(&lower);
    table().get(key).copied()
}

#[cfg(windows)]
pub fn is_media(vk: u16) -> bool {
    (0xA6..=0xB7).contains(&vk)
}
