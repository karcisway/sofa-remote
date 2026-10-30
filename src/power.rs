//! Power actions requested from the phone: shut down, restart, sleep, hibernate, lock, sign out,
//! screen off. The phone asks for confirmation before the dangerous ones (see web/app.js).

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Shutdown,
    Restart,
    Sleep,
    Hibernate,
    Lock,
    SignOut,
    ScreenOff,
}

impl Op {
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s.to_lowercase().as_str() {
            "shutdown" | "poweroff" => Self::Shutdown,
            "restart" | "reboot" => Self::Restart,
            "sleep" | "suspend" => Self::Sleep,
            "hibernate" => Self::Hibernate,
            "lock" => Self::Lock,
            "signout" | "logoff" | "logout" => Self::SignOut,
            "screenoff" | "displayoff" => Self::ScreenOff,
            _ => return None,
        })
    }
}

/// Run a helper program; its failure message (stderr) becomes the error shown on the phone.
fn command(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    let out = cmd.output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    Err(if err.is_empty() { format!("{program} failed ({})", out.status) } else { err.to_string() })
}

/// Sleeping blocks until the PC wakes up again, so it goes on its own thread, a moment later so
/// that the phone gets its answer first.
#[cfg(windows)]
fn later(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        f();
    });
}

#[cfg(windows)]
pub fn run(op: Op) -> Result<(), String> {
    use windows_sys::Win32::System::Power::SetSuspendState;
    use windows_sys::Win32::System::Shutdown::LockWorkStation;
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, WM_SYSCOMMAND};

    const SC_MONITORPOWER: usize = 0xF170;
    match op {
        Op::Shutdown => command("shutdown", &["/s", "/t", "0"]),
        Op::Restart => command("shutdown", &["/r", "/t", "0"]),
        Op::Hibernate => command("shutdown", &["/h"]),
        Op::SignOut => command("shutdown", &["/l"]),
        Op::Sleep => {
            later(|| unsafe {
                SetSuspendState(false, false, false);
            });
            Ok(())
        }
        Op::Lock => match unsafe { LockWorkStation() } {
            0 => Err("Windows refused to lock the PC".into()),
            _ => Ok(()),
        },
        Op::ScreenOff => {
            later(|| unsafe {
                // 2 = turn the monitors off; moving the mouse wakes them
                PostMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2);
            });
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
pub fn run(op: Op) -> Result<(), String> {
    match op {
        Op::Shutdown => command("systemctl", &["poweroff"]),
        Op::Restart => command("systemctl", &["reboot"]),
        Op::Sleep => command("systemctl", &["suspend"]),
        Op::Hibernate => command("systemctl", &["hibernate"]),
        Op::Lock => command("loginctl", &["lock-session"]),
        Op::SignOut => match std::env::var("XDG_SESSION_ID") {
            Ok(id) if !id.is_empty() => command("loginctl", &["terminate-session", &id]),
            _ => Err("No login session found to sign out of".into()),
        },
        Op::ScreenOff => command("xset", &["dpms", "force", "off"]),
    }
}

#[cfg(target_os = "macos")]
pub fn run(op: Op) -> Result<(), String> {
    let event = |name: &str| command("osascript", &["-e", &format!("tell application \"System Events\" to {name}")]);
    match op {
        Op::Shutdown => event("shut down"),
        Op::Restart => event("restart"),
        Op::Sleep => command("pmset", &["sleepnow"]),
        Op::Hibernate => Err("Hibernate isn't available on macOS".into()),
        // With "Require password after sleep" on, a sleeping display is locked
        Op::Lock | Op::ScreenOff => command("pmset", &["displaysleepnow"]),
        Op::SignOut => event("log out"),
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn run(_op: Op) -> Result<(), String> {
    Err("Power actions aren't supported on this system".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_aliases() {
        assert_eq!(Op::from_name("shutdown"), Some(Op::Shutdown));
        assert_eq!(Op::from_name("Reboot"), Some(Op::Restart));
        assert_eq!(Op::from_name("logoff"), Some(Op::SignOut));
        assert_eq!(Op::from_name("format-c"), None);
    }
}
