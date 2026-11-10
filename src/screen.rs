//! Screen snapshots for the phone's Screen tab: a list of monitors and a JPEG of one of them,
//! taken on request (no video stream). Monitor rectangles are in the same coordinates the mouse
//! uses (Windows: physical pixels of the virtual desktop, X11: root window pixels, macOS: points),
//! so the phone can point at a spot with numbers from 0 to 1 and the PC turns them into a position.

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub primary: bool,
}

impl Monitor {
    /// The point at (fx, fy) from the monitor's top-left corner, each from 0 to 1.
    pub fn point(&self, fx: f64, fy: f64) -> (i32, i32) {
        let px = self.x + (fx.clamp(0.0, 1.0) * (self.w.saturating_sub(1)) as f64).round() as i32;
        let py = self.y + (fy.clamp(0.0, 1.0) * (self.h.saturating_sub(1)) as f64).round() as i32;
        (px, py)
    }
    /// Where (x, y) is on this monitor (0 to 1), if it is on it.
    fn locate(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        let (dx, dy) = (x - self.x, y - self.y);
        (dx >= 0 && dy >= 0 && (dx as u32) < self.w && (dy as u32) < self.h)
            .then(|| (dx as f32 / self.w.max(1) as f32, dy as f32 / self.h.max(1) as f32))
    }
}

pub struct Frame {
    pub jpeg: Vec<u8>,
    /// Mouse pointer on this monitor (0 to 1), if it is there.
    pub cursor: Option<(f32, f32)>,
}

/// Raw pixels as the OS hands them over: 4 bytes per pixel, blue-green-red-unused.
struct Bgra {
    w: usize,
    h: usize,
    stride: usize,
    data: Vec<u8>,
}

/// Monitors from left to right (then top to bottom).
pub fn monitors() -> Vec<Monitor> {
    let mut list = os::monitors();
    list.sort_by_key(|m| (m.x, m.y));
    list
}

/// Where the mouse pointer is: the monitor's index (as in `monitors`) and the spot on it (0 to 1).
pub fn cursor() -> Option<(usize, f32, f32)> {
    let (x, y) = os::cursor()?;
    monitors().iter().enumerate().find_map(|(i, m)| m.locate(x, y).map(|(fx, fy)| (i, fx, fy)))
}

/// A JPEG of monitor `index`, made smaller (by a whole factor) to fit within max_w × max_h.
pub fn capture(index: usize, max_w: u32, max_h: u32, quality: u8) -> Result<Frame, String> {
    let list = monitors();
    let mon = list.get(index).or(list.first()).ok_or("no monitor found")?.clone();
    let raw = os::grab(&mon)?;
    let cursor = os::cursor().and_then(|(x, y)| mon.locate(x, y));
    let (w, h, rgb) = shrink(&raw, max_w.max(64) as usize, max_h.max(64) as usize);
    let mut jpeg = Vec::with_capacity(rgb.len() / 8);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(10, 95))
        .encode(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(Frame { jpeg, cursor })
}

/// Average k × k blocks (k the smallest whole factor that fits) into RGB.
fn shrink(src: &Bgra, max_w: usize, max_h: usize) -> (usize, usize, Vec<u8>) {
    let k = src.w.div_ceil(max_w).max(src.h.div_ceil(max_h)).max(1);
    let (w, h) = ((src.w / k).max(1), (src.h / k).max(1));
    let mut out = vec![0u8; w * h * 3];
    let n = (k * k) as u32;
    for oy in 0..h {
        for ox in 0..w {
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for y in oy * k..oy * k + k {
                let row = &src.data[y * src.stride + ox * k * 4..][..k * 4];
                for px in row.chunks_exact(4) {
                    b += px[0] as u32;
                    g += px[1] as u32;
                    r += px[2] as u32;
                }
            }
            let o = (oy * w + ox) * 3;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
        }
    }
    (w, h, out)
}

// ------------------------------------------------------------------ Windows (GDI)

#[cfg(windows)]
mod os {
    use super::{Bgra, Monitor};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    use windows_sys::core::BOOL;

    unsafe extern "system" fn each(mon: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        unsafe {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut info) != 0 {
                let r = info.rcMonitor;
                (*(out as *mut Vec<Monitor>)).push(Monitor {
                    x: r.left, y: r.top, w: (r.right - r.left) as u32, h: (r.bottom - r.top) as u32,
                    primary: info.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
                });
            }
        }
        1
    }

    pub fn monitors() -> Vec<Monitor> {
        let mut list: Vec<Monitor> = Vec::new();
        unsafe { EnumDisplayMonitors(null_mut(), null(), Some(each), &mut list as *mut _ as LPARAM) };
        list
    }

    pub fn grab(m: &Monitor) -> Result<Bgra, String> {
        let (w, h) = (m.w as i32, m.h as i32);
        unsafe {
            let screen = GetDC(null_mut());
            if screen.is_null() {
                return Err("can't read the screen".into());
            }
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            let copied = BitBlt(mem, 0, 0, w, h, screen, m.x, m.y, SRCCOPY) != 0;
            SelectObject(mem, old);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, // top-down rows
                biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed()
            };
            let mut data = vec![0u8; w as usize * h as usize * 4];
            let rows = if copied {
                GetDIBits(mem, bmp, 0, h as u32, data.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS)
            } else { 0 };
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(null_mut(), screen);
            if rows == 0 {
                return Err("can't read the screen (is the PC locked?)".into());
            }
            Ok(Bgra { w: w as usize, h: h as usize, stride: w as usize * 4, data })
        }
    }

    pub fn cursor() -> Option<(i32, i32)> {
        let mut p = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
    }
}

// ------------------------------------------------------------------ Linux (X11)

#[cfg(target_os = "linux")]
mod os {
    use super::{Bgra, Monitor};
    use std::sync::Mutex;
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};
    use x11rb::rust_connection::RustConnection;

    static CONN: Mutex<Option<(RustConnection, u32)>> = Mutex::new(None);

    fn with<T>(f: impl FnOnce(&RustConnection, u32) -> Option<T>) -> Option<T> {
        let mut guard = CONN.lock().unwrap();
        if guard.is_none() {
            let (c, n) = x11rb::connect(None).ok()?;
            let root = c.setup().roots[n].root;
            *guard = Some((c, root));
        }
        let (c, root) = guard.as_ref()?;
        let r = f(c, *root);
        if r.is_none() {
            *guard = None; // reconnect next time (the X server may have restarted)
        }
        r
    }

    pub fn monitors() -> Vec<Monitor> {
        with(|c, root| {
            if let Some(r) = c.randr_get_monitors(root, true).ok().and_then(|x| x.reply().ok()) {
                if !r.monitors.is_empty() {
                    return Some(r.monitors.iter().map(|m| Monitor {
                        x: m.x as i32, y: m.y as i32, w: m.width as u32, h: m.height as u32, primary: m.primary,
                    }).collect());
                }
            }
            let s = &c.setup().roots[0];
            Some(vec![Monitor { x: 0, y: 0, w: s.width_in_pixels as u32, h: s.height_in_pixels as u32, primary: true }])
        }).unwrap_or_default()
    }

    pub fn grab(m: &Monitor) -> Result<Bgra, String> {
        with(|c, root| {
            let r = c.get_image(ImageFormat::Z_PIXMAP, root, m.x as i16, m.y as i16, m.w as u16, m.h as u16, !0)
                .ok()?.reply().ok()?;
            let (w, h) = (m.w as usize, m.h as usize);
            (r.data.len() >= w * h * 4).then(|| Bgra { w, h, stride: r.data.len() / h, data: r.data })
        }).ok_or_else(|| "can't read the screen: an X11 session is needed (on Wayland, log in with Xorg)".into())
    }

    pub fn cursor() -> Option<(i32, i32)> {
        with(|c, root| {
            let p = c.query_pointer(root).ok()?.reply().ok()?;
            Some((p.root_x as i32, p.root_y as i32))
        })
    }
}

// ------------------------------------------------------------------ macOS (CoreGraphics)

#[cfg(target_os = "macos")]
mod os {
    use super::{Bgra, Monitor};
    use core_graphics::display::CGDisplay;
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    fn displays() -> Vec<(CGDisplay, Monitor)> {
        CGDisplay::active_displays().unwrap_or_default().into_iter().map(|id| {
            let d = CGDisplay::new(id);
            let b = d.bounds();
            let m = Monitor {
                x: b.origin.x as i32, y: b.origin.y as i32, w: b.size.width as u32, h: b.size.height as u32,
                primary: d.is_main(),
            };
            (d, m)
        }).collect()
    }

    pub fn monitors() -> Vec<Monitor> {
        displays().into_iter().map(|(_, m)| m).collect()
    }

    pub fn grab(m: &Monitor) -> Result<Bgra, String> {
        let (d, _) = displays().into_iter().find(|(_, x)| x == m).ok_or("monitor not found")?;
        let img = d.image().ok_or("can't read the screen: allow Screen Recording for this app in System Settings › Privacy & Security")?;
        if img.bits_per_pixel() != 32 {
            return Err("unsupported screen format".into());
        }
        let (w, h, stride) = (img.width(), img.height(), img.bytes_per_row());
        let data = img.data().bytes().to_vec();
        if data.len() < stride * h {
            return Err("can't read the screen".into());
        }
        Ok(Bgra { w, h, stride, data })
    }

    pub fn cursor() -> Option<(i32, i32)> {
        let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
        let p = CGEvent::new(src).ok()?.location();
        Some((p.x as i32, p.y as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinks_by_whole_factor() {
        // 4 × 2 pixels, left half white, right half black
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&[255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        }
        let src = Bgra { w: 4, h: 2, stride: 16, data };
        let (w, h, rgb) = shrink(&src, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgb, vec![255, 255, 255, 0, 0, 0]);
        assert_eq!(shrink(&src, 100, 100).0, 4);
    }

    #[test]
    fn monitor_points() {
        let m = Monitor { x: -1920, y: 0, w: 1920, h: 1080, primary: false };
        assert_eq!(m.point(0.0, 0.0), (-1920, 0));
        assert_eq!(m.point(1.0, 1.0), (-1, 1079));
        assert_eq!(m.locate(-960, 540), Some((0.5, 0.5)));
        assert_eq!(m.locate(0, 0), None);
    }
}
