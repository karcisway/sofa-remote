//! Virtual gamepads: every phone gets its own Xbox 360 controller, so games see one real
//! controller per player instead of a shared keyboard.
//!
//! - Windows: through the ViGEmBus driver (https://github.com/nefarius/ViGEmBus/releases).
//!   When it's missing, `driver::install` runs the official installer (built into release
//!   builds, else downloaded).
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
    fn open(&mut self) -> Result<Option<u32>, String> {
        self.retry_at = None;
        let state = self.state;
        let result = self.ensure().and_then(|pad| {
            pad.update(&state)?;
            Ok(pad.player())
        });
        if result.is_err() {
            self.pad = None;
        }
        result
    }

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
                let _ = reply.send(e.open());
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

/// Error text when the ViGEmBus driver isn't installed (the phone shows it).
#[cfg(windows)]
pub const DRIVER_MISSING: &str = "The PC needs the ViGEmBus gamepad driver";

/// Getting the ViGEmBus driver onto the PC: an installer the user already saved (next to
/// sofa-remote, in the current folder or in Downloads) is used first, then the one built into
/// the exe (release builds, see build.rs), else the official one is downloaded from GitHub. It's
/// accepted only when it's the known v1.22.0 file (SHA-256) or Windows says it's validly signed
/// by its publisher (Nefarius), then started; Windows asks for permission on the PC.
#[cfg(windows)]
pub mod driver {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    /// Where to download it: this project's releases carry a copy, then the publisher's own
    const URLS: [&str; 2] = [
        "https://github.com/karcisway/sofa-remote/releases/latest/download/ViGEmBus_1.22.0_x64_x86_arm64.exe",
        "https://github.com/nefarius/ViGEmBus/releases/download/v1.22.0/ViGEmBus_1.22.0_x64_x86_arm64.exe",
    ];
    // SHA-256 of that file (also in vendor/fetch.ps1)
    const SHA256: &str = "89220A7865076B342892F98865F3499FB7C4CFD673159E89D352C360FD014C6A";
    // Publisher of the ViGEmBus installer ("Nefarius Software Solutions e.U.")
    const SIGNER: &str = "Nefarius";

    /// The official installer, built in when vendor/ has it (BSD-3-Clause, vendor/LICENSE-ViGEmBus.txt)
    #[cfg(vigem_bundled)]
    pub(super) const BUNDLED: Option<&[u8]> = Some(include_bytes!("../vendor/ViGEmBus_1.22.0_x64_x86_arm64.exe"));
    #[cfg(not(vigem_bundled))]
    const BUNDLED: Option<&[u8]> = None;

    static RUNNING: AtomicBool = AtomicBool::new(false);
    /// After a failed attempt, gamepad use doesn't start another one for a while (no prompt or
    /// download every time a phone touches the pad).
    static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);
    const RETRY_AFTER: Duration = Duration::from_secs(120);

    pub fn installed() -> bool {
        vigem_client::Client::connect().is_ok()
    }

    /// Start `install` on its own thread unless one is already running or one failed less than
    /// `RETRY_AFTER` ago; `done` gets the result.
    pub fn install_in_background(done: impl FnOnce(Result<(), String>) + Send + 'static) -> bool {
        if FAILED_AT.lock().unwrap().is_some_and(|t| t.elapsed() < RETRY_AFTER) || RUNNING.swap(true, Ordering::SeqCst) {
            return false;
        }
        std::thread::spawn(move || {
            let r = install();
            *FAILED_AT.lock().unwrap() = r.is_err().then(Instant::now);
            RUNNING.store(false, Ordering::SeqCst);
            done(r);
        });
        true
    }

    pub fn install() -> Result<(), String> {
        let (file, downloaded) = match find_saved() {
            Some(f) => {
                println!("Gamepads: using the driver installer {}", f.display());
                (f, false)
            }
            None => (match BUNDLED {
                Some(bytes) => unpack(bytes)?,
                None => download()?,
            }, true),
        };
        if let Err(e) = verify(&file) {
            if downloaded {
                let _ = std::fs::remove_file(&file);
            }
            return Err(e);
        }
        println!("Gamepads: starting the driver installer; approve the Windows prompt on this PC.");
        // A declined prompt is error 1223; anything else is reported as PowerShell words it
        let code = powershell(&format!(
            "try {{ (Start-Process -FilePath '{}' -Verb RunAs -Wait -PassThru).ExitCode }} \
             catch {{ $e = $_.Exception; while ($e.InnerException) {{ $e = $e.InnerException }}; \
             if ($e.NativeErrorCode -eq 1223) {{ 'cancelled' }} else {{ 'error: ' + $e.Message }} }}",
            file.display()))?;
        let result = match code.as_str() {
            "0" | "3010" if installed() => Ok(()),
            "0" | "3010" => Err("driver installed: restart the PC to use gamepads".into()),
            "cancelled" | "1223" => Err("gamepad driver install was cancelled on the PC (the Windows prompt was declined)".into()),
            other => Err(format!("gamepad driver installer failed ({})", other.trim_start_matches("error: "))),
        };
        if matches!(code.as_str(), "0" | "3010") {
            if downloaded {
                let _ = std::fs::remove_file(&file);
            }
            return result;
        }
        // Left in place so it can be started by hand
        result.map_err(|e| format!("{e}. To install it yourself, run {}", file.display()))
    }

    /// A ViGEmBus installer the user saved: next to sofa-remote, in the current folder or in Downloads.
    fn find_saved() -> Option<PathBuf> {
        let dirs = [
            std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)),
            std::env::current_dir().ok(),
            std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Downloads")),
        ];
        dirs.into_iter().flatten().find_map(|dir| {
            std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
                name.starts_with("vigembus") && name.ends_with(".exe")
            })
        })
    }

    fn temp_file() -> Result<PathBuf, String> {
        let dir = std::env::temp_dir().join("sofa-remote");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir.join("ViGEmBus_1.22.0_x64_x86_arm64.exe"))
    }

    /// Write the built-in installer to the temp folder.
    pub(super) fn unpack(bytes: &[u8]) -> Result<PathBuf, String> {
        let file = temp_file()?;
        std::fs::write(&file, bytes).map_err(|e| format!("couldn't unpack the gamepad driver: {e}"))?;
        Ok(file)
    }

    /// Download the official installer to the temp folder from `URLS`: curl.exe (Windows 10 1803+)
    /// first, then PowerShell, which goes through the system proxy settings. curl skips the
    /// certificate revocation check, which fails on PCs that can't reach the revocation servers
    /// (the file is checked by its hash anyway).
    fn download() -> Result<PathBuf, String> {
        let file = temp_file()?;
        println!("Gamepads: downloading the ViGEmBus driver installer...");
        for url in URLS {
            let curl = Command::new("curl.exe").args(["-fsL", "--ssl-no-revoke", "--retry", "2", "-o"]).arg(&file)
                .arg(url).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
            if curl || powershell(&format!(
                "try {{ [Net.ServicePointManager]::SecurityProtocol = 'Tls12'; Invoke-WebRequest -UseBasicParsing -Uri '{url}' -OutFile '{}'; 'ok' }} catch {{ 'failed' }}",
                file.display())).is_ok_and(|r| r == "ok") {
                return Ok(file);
            }
        }
        let _ = std::fs::remove_file(&file);
        Err(format!("couldn't download the gamepad driver from github.com. Get it on any device from \
             {} and run it on this PC, or save it next to sofa-remote and use a gamepad again.", URLS[0]))
    }

    /// The known v1.22.0 installer is accepted by its hash; anything else needs the signature. The
    /// hash comes first because the signature check can fail on PCs that can't reach the
    /// certificate revocation servers (CRYPT_E_REVOCATION_OFFLINE).
    pub(super) fn verify(file: &Path) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(file).map_err(|e| format!("couldn't read the driver installer: {e}"))?;
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02X}")).collect();
        if hash == SHA256 { Ok(()) } else { check_signature(file) }
    }

    /// Accept the file only with a valid Authenticode signature from the driver's publisher.
    pub(super) fn check_signature(file: &Path) -> Result<(), String> {
        let out = powershell(&format!(
            "$s = Get-AuthenticodeSignature -LiteralPath '{}'; \"$($s.Status)|$($s.SignerCertificate.Subject)\"",
            file.display()))?;
        let (status, subject) = out.split_once('|').unwrap_or((&out, ""));
        if status == "Valid" && subject.contains(SIGNER) {
            Ok(())
        } else {
            Err(format!("the downloaded driver installer isn't signed by {SIGNER} ({status}); not running it"))
        }
    }

    fn powershell(script: &str) -> Result<String, String> {
        // A PSModulePath inherited from PowerShell 7 can make Windows PowerShell load the wrong
        // modules: let it use its own
        let out = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env_remove("PSModulePath").output().map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if stdout.is_empty() && !out.stderr.is_empty() {
            return Err(format!("PowerShell: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(stdout)
    }
}

#[cfg(windows)]
mod platform {
    use super::{PadState, VirtualPad};
    use std::time::{Duration, Instant};
    use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

    struct Pad(Xbox360Wired<Client>);

    pub(super) fn submit_with_retry(mut submit: impl FnMut() -> Result<(), vigem_client::Error>) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            match submit() {
                Ok(()) => return Ok(()),
                // ViGEmBus returns ERROR_NO_MORE_ITEMS when XInput has not queued
                // its next USB read yet. Keep the controller and retry the report,
                // including button releases, instead of unplugging it.
                Err(vigem_client::Error::WinError(259) | vigem_client::Error::TargetNotReady)
                    if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    pub(super) fn connect_error(error: vigem_client::Error) -> String {
        match error {
            vigem_client::Error::BusNotFound => super::DRIVER_MISSING.to_string(),
            vigem_client::Error::BusVersionMismatch =>
                "The installed ViGEmBus driver is incompatible. Repair ViGEmBus 1.22.0 and restart the PC".into(),
            other => format!("Cannot connect to the installed ViGEmBus driver: {other}"),
        }
    }

    pub fn create() -> Result<Box<dyn VirtualPad>, String> {
        let client = Client::connect().map_err(connect_error)?;
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
            submit_with_retry(|| self.0.update(&g))
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
    fn failed_open_discards_a_stale_controller() {
        struct Stale;
        impl VirtualPad for Stale {
            fn update(&mut self, _: &PadState) -> Result<(), String> { Err("controller disconnected".into()) }
            fn player(&mut self) -> Option<u32> { Some(1) }
        }
        let mut entry = Entry { pad: Some(Box::new(Stale)), ..Default::default() };
        assert_eq!(entry.open(), Err("controller disconnected".into()));
        assert!(entry.pad.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn installed_driver_errors_do_not_request_reinstallation() {
        use vigem_client::Error;
        assert_eq!(platform::connect_error(Error::BusNotFound), DRIVER_MISSING);
        for error in [Error::BusAccessFailed(5), Error::WinError(31), Error::BusVersionMismatch] {
            let message = platform::connect_error(error);
            assert_ne!(message, DRIVER_MISSING);
            assert!(message.contains("ViGEmBus"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn busy_xinput_queue_retries_the_report_without_unplugging() {
        use vigem_client::Error;
        let mut results = [Err(Error::WinError(259)), Err(Error::TargetNotReady), Ok(())].into_iter();
        assert_eq!(platform::submit_with_retry(|| results.next().unwrap()), Ok(()));
        assert_eq!(platform::submit_with_retry(|| Err(Error::WinError(5))), Err("win error: 5".into()));
        assert_eq!(platform::submit_with_retry(|| Err(Error::WinError(259))), Err("win error: 259".into()));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the installed ViGEmBus driver and two free XInput slots"]
    fn windows_controller_reports_input() {
        #[repr(C)]
        #[derive(Default)]
        struct State {
            packet: u32,
            gamepad: vigem_client::XGamepad,
        }
        #[link(name = "xinput1_4")]
        unsafe extern "system" {
            fn XInputGetState(index: u32, state: *mut State) -> u32;
        }
        let mut pad = platform::create().expect("create virtual controller");
        let mut other = platform::create().expect("create a second virtual controller");
        let other_state = PadState { buttons: 0x2000, rt: 77, rx: 12345, ..Default::default() };
        other.update(&other_state).expect("send second controller state");
        let deadline = Instant::now() + Duration::from_secs(2);
        let other_index = loop {
            let found = (0..4).find(|&slot| {
                let mut actual = State::default();
                (unsafe { XInputGetState(slot, &mut actual) }) == 0
                    && actual.gamepad.buttons.raw == other_state.buttons
                    && actual.gamepad.right_trigger == other_state.rt
                    && actual.gamepad.thumb_rx == other_state.rx
            });
            if let Some(slot) = found { break slot; }
            assert!(Instant::now() < deadline, "second controller did not receive input");
            std::thread::sleep(Duration::from_millis(10));
        };
        let reported_player = pad.player();
        let mut index = None;
        let pressed = PadState { buttons: 0x1000, lt: 123, lx: -12345, ..Default::default() };
        for (report, expected) in [pressed, PadState::default()].into_iter().cycle().take(128).enumerate() {
            pad.update(&expected).expect("send controller state");
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if index.is_none() {
                    index = (0..4).find(|&slot| {
                        let mut actual = State::default();
                        (unsafe { XInputGetState(slot, &mut actual) }) == 0
                            && actual.gamepad.buttons.raw == pressed.buttons
                            && actual.gamepad.left_trigger == pressed.lt
                            && actual.gamepad.thumb_lx == pressed.lx
                    });
                }
                let mut actual = State::default();
                let result = index.map(|slot| unsafe { XInputGetState(slot, &mut actual) }).unwrap_or(1167);
                if result == 0 && actual.gamepad.buttons.raw == expected.buttons
                    && actual.gamepad.left_trigger == expected.lt && actual.gamepad.thumb_lx == expected.lx {
                    break;
                }
                assert!(Instant::now() < deadline, "XInput slot {index:?} did not receive report {report}, {expected:?}: {result}, packet {}, {:?}", actual.packet, actual.gamepad);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if let Some(player) = reported_player {
            assert_eq!(Some(player - 1), index);
        }
        assert_ne!(index, Some(other_index));
        let mut actual = State::default();
        assert_eq!(unsafe { XInputGetState(other_index, &mut actual) }, 0);
        assert_eq!(actual.gamepad.buttons.raw, other_state.buttons);
        assert_eq!(actual.gamepad.right_trigger, other_state.rt);
        assert_eq!(actual.gamepad.thumb_rx, other_state.rx);
        other.update(&PadState::default()).expect("release second controller");
    }

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

    #[cfg(windows)]
    #[test]
    fn driver_installer_must_be_signed_by_its_publisher() {
        // validly signed, but by Microsoft
        let curl = std::path::Path::new(r"C:\Windows\System32\curl.exe");
        assert!(driver::check_signature(curl).unwrap_err().contains("isn't signed"));
        let unsigned = std::env::temp_dir().join("sofa-remote-unsigned-test.exe");
        std::fs::write(&unsigned, b"not a program").unwrap();
        assert!(driver::check_signature(&unsigned).is_err());
        let _ = std::fs::remove_file(unsigned);
    }

    #[cfg(vigem_bundled)]
    #[test]
    fn bundled_driver_installer_is_accepted() {
        let file = driver::unpack(driver::BUNDLED.unwrap()).unwrap();
        assert_eq!(driver::verify(&file), Ok(()));
        let _ = std::fs::remove_file(file);
    }
}
