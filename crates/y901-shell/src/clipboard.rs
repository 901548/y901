//! 平台无关的**系统剪贴板**。
//!
//! ## 三个平台的实现差异很大
//!
//! | 平台 | 机制 | 形态 |
//! |---|---|---|
//! | Windows | `OpenClipboard` / `SetClipboardData(CF_UNICODETEXT)` | 同步 API |
//! | Linux | X11 Selections（`SelectionRequest` 必须实时应答） | **必须独立线程** |
//! | macOS | `NSPasteboard` | 本轮未实现 |
//!
//! 业务层只看到 `Clipboard` trait，不知道上面这些差异。

use std::sync::Arc;

/// 系统剪贴板接口。
///
/// 为什么是 `&self` 而不是 `&mut self`：三个平台的实现都是"内部可变"（线程句柄 / 系统调用），
/// 对外取 `&self` 让它能被 `Arc` 共享到 UI 与后台线程。
pub trait Clipboard: Send + Sync {
    /// 读取剪贴板文本（UTF-8）。
    fn get_text(&self) -> Result<String, String>;

    /// 写入剪贴板文本（UTF-8）。
    fn set_text(&self, text: &str) -> Result<(), String>;
}

/// 拿到当前平台的系统剪贴板。
///
/// 失败不会 panic：实现不能启动时返回一个**总是报错的剪贴板**，
/// 让 UI 的 Ctrl+V 失败而不是崩。
pub fn system_clipboard() -> Arc<dyn Clipboard> {
    #[cfg(target_os = "windows")]
    {
        // 这里也保留一个可选的纯内存实现入口（便于测试与无剪贴板环境）
        if std::env::var_os("Y901_NO_CLIPBOARD").is_some() {
            return Arc::new(NullClipboard(
                "剪贴板已被 Y901_NO_CLIPBOARD 禁用".to_string(),
            ));
        }
        Arc::new(win::WinClipboard)
    }
    #[cfg(target_os = "linux")]
    {
        match linux::LinuxClipboard::start() {
            Ok(c) => Arc::new(c),
            Err(e) => {
                eprintln!("[y901] 剪贴板不可用：{e}");
                Arc::new(NullClipboard(e))
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Arc::new(NullClipboard("当前平台剪贴板待实现".to_string()))
    }
}

/// 不可用时的兆底：每次调用都报同一个错误。
struct NullClipboard(String);

impl Clipboard for NullClipboard {
    fn get_text(&self) -> Result<String, String> {
        Err(self.0.clone())
    }
    fn set_text(&self, _text: &str) -> Result<(), String> {
        Err(self.0.clone())
    }
}

// ============================ Windows ============================
#[cfg(target_os = "windows")]
mod win {
    use super::Clipboard;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };

    /// `CF_UNICODETEXT`
    const CF_UNICODETEXT: u32 = 13;

    /// 进程内互斥：剪贴板是**系统全局单例**，同进程多线程并发访问会把对方手里的
    /// 句柄底下的内存释放掉 → `STATUS_HEAP_CORRUPTION`（实测崩在并行跑两个测试时）。
    static CLIP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Windows 剪贴板句柄（无状态，每次调用开关一次）。
    pub struct WinClipboard;

    impl Clipboard for WinClipboard {
        fn get_text(&self) -> Result<String, String> {
            let _guard = CLIP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            open_with_retry()?;
            // SAFETY: 剪贴板已打开；句柄属于剪贴板，在 CloseClipboard 之前有效。
            let s = unsafe {
                let h = GetClipboardData(CF_UNICODETEXT);
                if h.is_null() {
                    CloseClipboard();
                    return Ok(String::new()); // 空剪贴板 = 空字符串，不是错误
                }
                let p = windows_sys::Win32::System::Memory::GlobalLock(h) as *const u16;
                if p.is_null() {
                    CloseClipboard();
                    return Err("剪贴板内存锁定失败".to_string());
                }
                let mut len = 0usize;
                while *p.add(len) != 0 {
                    len += 1;
                }
                let v = std::slice::from_raw_parts(p, len).to_vec();
                windows_sys::Win32::System::Memory::GlobalUnlock(h);
                CloseClipboard();
                v
            };
            // SAFETY: `s` 是一个独立的 UTF-16 Vec，与剪贴板已无关。
            Ok(String::from_utf16_lossy(&s))
        }

        fn set_text(&self, text: &str) -> Result<(), String> {
            let _guard = CLIP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = wide.len() * 2;
            open_with_retry()?;
            // SAFETY: 开发剪贴板后才能 Empty/Set；内存与剪贴板生命周期由系统接管。
            let r = unsafe {
                if EmptyClipboard() == 0 {
                    CloseClipboard();
                    return Err(format!(
                        "EmptyClipboard 失败（错误 {}）",
                        std::io::Error::last_os_error()
                    ));
                }
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
                if h.is_null() {
                    CloseClipboard();
                    return Err("剪贴板内存分配失败".to_string());
                }
                let p = GlobalLock(h) as *mut u16;
                if p.is_null() {
                    CloseClipboard();
                    return Err("剪贴板内存锁定失败".to_string());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                GlobalUnlock(h);
                if SetClipboardData(CF_UNICODETEXT, h as HANDLE).is_null() {
                    CloseClipboard();
                    return Err(format!(
                        "SetClipboardData 失败（错误 {}）",
                        std::io::Error::last_os_error()
                    ));
                }
                CloseClipboard();
                Ok::<(), String>(())
            };
            r
        }
    }

    /// 开剪贴板，失败重试 3 次（间隔 10ms）。
    ///
    /// 为什么需要重试：剪贴板是**全局独占资源**，
    /// 另一个进程正在用时 `OpenClipboard` 会失败，几十毫秒后就好了。
    fn open_with_retry() -> Result<(), String> {
        let mut last = String::new();
        for attempt in 0..3 {
            // SAFETY: 传 null 表示与当前任务关联。
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(());
            }
            last = format!(
                "OpenClipboard 失败（错误 {}）",
                std::io::Error::last_os_error()
            );
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Err(last)
    }
}

// ============================ Linux / X11 ============================
#[cfg(target_os = "linux")]
mod linux {
    use super::Clipboard;
    use y901_backend_x11::clipboard as x11_bridge;

    /// 基于 X11 Selections 的剪贴板（内部是一个独立线程）。
    pub struct LinuxClipboard {
        inner: x11_bridge::X11Clipboard,
    }

    impl LinuxClipboard {
        /// 启动剪贴板线程（自己的 X 连接 + 隐藏 1×1 窗口）。
        pub fn start() -> Result<Self, String> {
            let display = std::env::var("DISPLAY").unwrap_or_default();
            if display.is_empty() {
                return Err("DISPLAY 未设置".to_string());
            }
            Ok(Self {
                inner: x11_bridge::X11Clipboard::start(&display)?,
            })
        }
    }

    impl Clipboard for LinuxClipboard {
        fn get_text(&self) -> Result<String, String> {
            self.inner.get(x11_bridge::Selection::Clipboard)
        }
        fn set_text(&self, text: &str) -> Result<(), String> {
            self.inner.set(x11_bridge::Selection::Clipboard, text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_on_supported_platforms() {
        let cb = system_clipboard();
        let text = "y901 剪贴板测试 ABC 123 中文";
        if cb.set_text(text).is_err() {
            // 不支持的平台 / 无 X 会话：照旧不能假装通过，但也不能因环境而假红
            eprintln!("[跳过] 本环境剪贴板不可用");
            return;
        }
        assert_eq!(cb.get_text().expect("读取"), text);
    }

    #[test]
    fn get_text_on_empty_clipboard_is_ok_or_err_but_never_panics() {
        let cb = system_clipboard();
        let _ = cb.get_text();
    }
}
