//! Native Windows UTF-16 clipboard access (`OpenClipboard` / `CF_UNICODETEXT`),
//! with safe cross-platform fallbacks for non-Windows builds.

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    pub fn get_clipboard_text() -> Option<String> {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return None;
            }
            let h = GetClipboardData(CF_UNICODETEXT);
            if h.is_null() {
                CloseClipboard();
                return None;
            }
            let ptr = GlobalLock(h) as *const u16;
            if ptr.is_null() {
                CloseClipboard();
                return None;
            }
            let mut len = 0usize;
            while len < 1_048_576 && *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            GlobalUnlock(h);
            CloseClipboard();
            if text.is_empty() { None } else { Some(text) }
        }
    }

    pub fn set_clipboard_text(text: &str) -> bool {
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        utf16.push(0);
        let bytes = utf16.len() * std::mem::size_of::<u16>();
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return false;
            }
            EmptyClipboard();
            let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if hmem.is_null() {
                CloseClipboard();
                return false;
            }
            let dst = GlobalLock(hmem) as *mut u16;
            if dst.is_null() {
                CloseClipboard();
                return false;
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
            GlobalUnlock(hmem);
            let ok = !SetClipboardData(CF_UNICODETEXT, hmem).is_null();
            CloseClipboard();
            ok
        }
    }
}

#[cfg(target_os = "windows")]
pub use imp::{get_clipboard_text, set_clipboard_text};

#[cfg(not(target_os = "windows"))]
pub fn get_clipboard_text() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn set_clipboard_text(_text: &str) -> bool {
    false
}
