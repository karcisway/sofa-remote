//! Tells the phones when a text field has keyboard focus on the PC, so they can open their
//! keyboard by themselves. Windows only (UI Automation); elsewhere it never reports anything.

/// Calls `on_change` with `true`/`false` whenever the focused element becomes / stops being a
/// text field. Runs on its own thread.
#[cfg(windows)]
pub fn watch(on_change: impl Fn(bool) + Send + 'static) {
    use std::time::Duration;
    use windows::Win32::System::Variant::VT_BOOL;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, UIA_ComboBoxControlTypeId, UIA_DocumentControlTypeId,
        UIA_EditControlTypeId, UIA_ValueIsReadOnlyPropertyId,
    };

    // Text boxes, and documents that say they are editable (Notepad, Word). A web page is a
    // document too, but read-only unless it's an editor.
    fn is_text(e: &IUIAutomationElement) -> bool {
        unsafe {
            if !e.CurrentIsEnabled().is_ok_and(|b| b.as_bool()) {
                return false;
            }
            let kind = e.CurrentControlType().unwrap_or_default();
            let read_only = || {
                let v = e.GetCurrentPropertyValue(UIA_ValueIsReadOnlyPropertyId).ok()?;
                let v = &v.Anonymous.Anonymous;
                (v.vt == VT_BOOL).then(|| v.Anonymous.boolVal.as_bool())
            };
            match kind {
                k if k == UIA_EditControlTypeId => read_only() != Some(true),
                k if k == UIA_ComboBoxControlTypeId => read_only() == Some(false),
                k if k == UIA_DocumentControlTypeId => read_only() == Some(false),
                _ => false,
            }
        }
    }

    let spawned = std::thread::Builder::new().name("focus".into()).spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let uia: IUIAutomation = match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
            Ok(u) => u,
            Err(e) => return eprintln!("Text-field detection unavailable: {e}"),
        };
        let mut last = false;
        loop {
            // Polled (4×/s): one cheap cross-process call, simpler than a COM event handler
            let text = uia.GetFocusedElement().is_ok_and(|e| is_text(&e));
            if text != last {
                last = text;
                on_change(text);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    if let Err(e) = spawned {
        eprintln!("Text-field detection unavailable: {e}");
    }
}

#[cfg(not(windows))]
pub fn watch(_on_change: impl Fn(bool) + Send + 'static) {}
