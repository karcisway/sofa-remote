//! Windows backend: SendInput with hardware scan codes, so games that read DirectInput /
//! raw input (and ignore virtual-key-only events) see the keys too.
//!
//! While the PC is locked (or showing a UAC prompt) the keyboard belongs to another desktop that
//! a normal process can't send input to. When SendInput starts failing, the input thread moves
//! onto the desktop that currently receives input. Windows only lets SYSTEM do that, so the
//! thread borrows winlogon.exe's token, which works when sofa-remote runs as administrator.

use super::{Button, Platform};
use crate::keys::is_media;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, DuplicateTokenEx, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SecurityImpersonation, TOKEN_ADJUST_PRIVILEGES, TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenImpersonation,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetThreadDesktop, GetUserObjectInformationW, HDESK, OpenInputDesktop, SetThreadDesktop, UOI_NAME,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, SetThreadToken,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

const EXTENDED: &[u16] = &[0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2C, 0x2D, 0x2E,
                           0x5B, 0x5C, 0x5D, 0x6F, 0xA3, 0xA5];

const GENERIC_ALL: u32 = 0x1000_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const DESKTOP_CHECK_EVERY: Duration = Duration::from_millis(250);

pub struct Backend {
    /// Desktop this thread moved to (null while it is still on the one it started on).
    desktop: HDESK,
    desktop_name: String,
    checked: Instant,
    /// Already tried to run as SYSTEM (needed to open the lock screen's desktop; once is enough).
    tried_system: bool,
    warned: bool,
}

impl Backend {
    pub fn new() -> Result<Self, String> {
        let desktop_name = desktop_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) });
        Ok(Backend { desktop: null_mut(), desktop_name, checked: Instant::now(), tried_system: false, warned: false })
    }

    fn send(&mut self, inputs: &[INPUT]) {
        self.follow_input_desktop(false);
        if !self.inject(inputs) {
            // Refused: probably locked, so the input desktop changed. Follow it and try once more.
            self.follow_input_desktop(true);
            self.inject(inputs);
        }
    }

    fn inject(&self, inputs: &[INPUT]) -> bool {
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) != 0 }
    }

    /// Attach this thread to the desktop that gets the keyboard now (Default, or Winlogon when
    /// locked). Checked at most every 250 ms unless `force`.
    fn follow_input_desktop(&mut self, force: bool) {
        if !force && self.checked.elapsed() < DESKTOP_CHECK_EVERY {
            return;
        }
        self.checked = Instant::now();
        let mut desk = open_input_desktop();
        if desk.is_null() && !self.tried_system {
            self.tried_system = true;
            impersonate_system();
            desk = open_input_desktop();
        }
        if desk.is_null() {
            if !self.warned {
                self.warned = true;
                eprintln!("Can't send input to the lock screen: run sofa-remote as administrator to type there");
                super::lock_blocked();
            }
            return;
        }
        self.warned = false;
        let name = desktop_name(desk);
        if name == self.desktop_name {
            unsafe { CloseDesktop(desk) };
            return;
        }
        if unsafe { SetThreadDesktop(desk) } == 0 {
            unsafe { CloseDesktop(desk) };
            return;
        }
        let old = std::mem::replace(&mut self.desktop, desk);
        self.desktop_name = name;
        if !old.is_null() {
            unsafe { CloseDesktop(old) };
        }
    }
}

fn open_input_desktop() -> HDESK {
    unsafe { OpenInputDesktop(0, 0, GENERIC_ALL) }
}

fn desktop_name(desk: HDESK) -> String {
    let mut buf = [0u16; 128];
    let mut bytes = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(desk, UOI_NAME, buf.as_mut_ptr().cast(), size_of_val(&buf) as u32, &mut bytes)
    };
    if ok == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)])
}

/// Process id of this session's winlogon.exe.
fn winlogon_pid() -> Option<u32> {
    unsafe {
        let mut session = 0;
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry);
        while more != 0 {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case("winlogon.exe") {
                let mut s = 0;
                if ProcessIdToSessionId(entry.th32ProcessID, &mut s) != 0 && s == session {
                    found = Some(entry.th32ProcessID);
                    break;
                }
            }
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        found
    }
}

/// Make the calling thread run as SYSTEM with winlogon.exe's token. Needs administrator rights
/// (SeDebugPrivilege); returns false without them.
fn impersonate_system() -> bool {
    unsafe {
        let mut own: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut own) == 0 {
            return false;
        }
        let name: Vec<u16> = "SeDebugPrivilege\0".encode_utf16().collect();
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        let enabled = LookupPrivilegeValueW(null(), name.as_ptr(), &mut luid) != 0 && {
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
            };
            AdjustTokenPrivileges(own, 0, &privileges, 0, null_mut(), null_mut()) != 0
                && GetLastError() != ERROR_NOT_ALL_ASSIGNED
        };
        CloseHandle(own);
        let Some(pid) = winlogon_pid().filter(|_| enabled) else { return false };

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut token: HANDLE = null_mut();
        let mut copy: HANDLE = null_mut();
        let ok = OpenProcessToken(process, TOKEN_DUPLICATE | TOKEN_QUERY, &mut token) != 0
            && DuplicateTokenEx(token, MAXIMUM_ALLOWED, null(), SecurityImpersonation, TokenImpersonation, &mut copy) != 0
            && SetThreadToken(null(), copy) != 0;
        for h in [copy, token, process] {
            if !h.is_null() {
                CloseHandle(h);
            }
        }
        ok
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
        self.send(&[keyboard(vk, scan, flags)]);
    }

    fn text(&mut self, text: &str) {
        let mut inputs = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            inputs.push(keyboard(0, unit, KEYEVENTF_UNICODE));
            inputs.push(keyboard(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        self.send(&inputs);
    }

    fn move_rel(&mut self, dx: i32, dy: i32) {
        self.send(&[mouse(MOUSEEVENTF_MOVE, dx, dy, 0)]);
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
        self.send(&[mouse(flags, 0, 0, 0)]);
    }

    fn scroll(&mut self, dy: i32, dx: i32) {
        if dy != 0 {
            self.send(&[mouse(MOUSEEVENTF_WHEEL, 0, 0, dy)]);
        }
        if dx != 0 {
            self.send(&[mouse(MOUSEEVENTF_HWHEEL, 0, 0, dx)]);
        }
    }
}
