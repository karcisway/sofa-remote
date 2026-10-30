//! Administrator rights (Windows). sofa-remote works without them; with them it can also type and
//! click on the lock screen and on UAC prompts (see input/windows.rs). A phone can ask for a
//! restart with a permission prompt on the PC. That choice is remembered, so the next start asks
//! again by itself.

use std::path::{Path, PathBuf};

fn pref_file(data_dir: &Path) -> PathBuf {
    data_dir.join(".remote_admin")
}

/// Did the user choose to run as administrator?
pub fn wanted(data_dir: &Path) -> bool {
    pref_file(data_dir).exists()
}

pub fn set_wanted(data_dir: &Path, wanted: bool) {
    let file = pref_file(data_dir);
    if wanted {
        let _ = std::fs::write(file, "1");
    } else {
        let _ = std::fs::remove_file(file);
    }
}

/// Flags that only this feature uses; they aren't passed on to the restarted program.
pub const FLAG_RESTARTED: &str = "--restarted";
pub const FLAG_NO_ELEVATE: &str = "--no-elevate";

#[cfg(windows)]
pub const SUPPORTED: bool = true;
#[cfg(not(windows))]
pub const SUPPORTED: bool = false;

#[cfg(windows)]
pub fn elevated() -> bool {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0;
        let ok = GetTokenInformation(token, TokenElevation, (&mut info as *mut TOKEN_ELEVATION).cast(),
                                     size_of::<TOKEN_ELEVATION>() as u32, &mut size) != 0;
        CloseHandle(token);
        ok && info.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn elevated() -> bool {
    false
}

/// One argument for a command line, quoted when it needs it.
#[cfg_attr(not(windows), allow(dead_code))]
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('"', "\\\""))
}

/// Start this program again with a Windows permission prompt (UAC). Ok means it was approved and
/// the new one is starting: the caller should quit. Err: refused, or it couldn't start.
#[cfg(windows)]
pub fn relaunch_elevated() -> Result<(), String> {
    use std::ptr::null;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != FLAG_RESTARTED && a != FLAG_NO_ELEVATE)
        .map(|a| quote(&a))
        .collect();
    args.push(FLAG_RESTARTED.into());
    let dir = std::env::current_dir().ok().map(|d| wide(&d.to_string_lossy()));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("runas").as_ptr(),
            wide(&exe.to_string_lossy()).as_ptr(),
            wide(&args.join(" ")).as_ptr(),
            dir.as_ref().map_or(null(), |d| d.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value above 32 on success
    if result as isize > 32 { Ok(()) } else { Err("Windows didn't give administrator rights".into()) }
}

#[cfg(not(windows))]
pub fn relaunch_elevated() -> Result<(), String> {
    Err("Administrator rights are only used on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(quote("--port"), "--port");
        assert_eq!(quote("My PC"), "\"My PC\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
}
