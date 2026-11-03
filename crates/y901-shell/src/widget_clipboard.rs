//! 把系统剪贴板适配成 `y901_widgets::TextInput` 要求的形状。
//!
//! ## 为什么需要这一层
//!
//! `TextInput` 的 `Clipboard` trait 是 `get() -> Option<String>` / `set(&str)`，
//! 它不认识 `Result`——也不应该认识（控件不该处理系统错误）。
//! 这里把错误吞成 `None`，让 Ctrl+V 失败得**安静**（而不是弹错误）。
//!
//! ## 为什么不直接让 `TextInput` 依赖 `y901-shell`
//!
//! 依赖方向是 `shell → widgets`：控件库必须能被任何人单独使用，
//! 不能反过来被某个具体平台层绑定。

use std::sync::Arc;

use crate::clipboard::{system_clipboard, Clipboard};

/// 系统剪贴板 → `TextInput` 剪贴板 的适配器。
pub struct WidgetClipboard {
    /// 底层系统剪贴板（可在多个输入框之间共享）
    inner: Arc<dyn Clipboard>,
}

impl WidgetClipboard {
    /// 用系统剪贴板构造（常用入口）。
    ///
    /// 副作用：Linux 下会在首次调用时启动剪贴板线程。
    pub fn system() -> Self {
        Self {
            inner: system_clipboard(),
        }
    }
}

impl y901_widgets::text_input::Clipboard for WidgetClipboard {
    fn get(&mut self) -> Option<String> {
        match self.inner.get_text() {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    fn set(&mut self, text: &str) {
        let _ = self.inner.set_text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_widgets::text_input::Clipboard as WidgetClipboardTrait;

    /// 系统剪贴板是**进程外的全局资源**，同一个测试二进制里多个用例并行写它就会互相覆盖。
    /// 用一把进程内锁把"写 → 读" 变成原子区，否则会出现假红
    /// （实际遇到过：读回来的是另一个测试写的串）。
    static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn adapter_forwards_to_system_clipboard() {
        let _guard = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut c = WidgetClipboard::system();
        let text = "adapter 测试 ABC 中文";
        c.set(text);
        // 读不回来也只能说明环境不支持（无 X 会话等），不能当作失败
        if let Some(got) = c.get() {
            assert_eq!(got, text, "适配器应原样转发");
        }
    }

    #[test]
    fn empty_clipboard_maps_to_none() {
        let _guard = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 空剪贴板对控件来说就是"没有东西可粘"
        let mut c = WidgetClipboard::system();
        c.set("");
        assert_eq!(c.get(), None);
    }
}
