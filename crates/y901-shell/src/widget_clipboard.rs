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

    /// 系统剪贴板是**进程外的全局资源**：与 `clipboard.rs` 的用例共用同一把进程锁，
    /// 否则两组用例并行时会互相覆盖（实测：单模块加锁仍偶发红，根因就是锁不共享）。
    use crate::clipboard::CLIPBOARD_TEST_LOCK as CLIPBOARD_LOCK;

    /// 写入后**重试读取**，并在每次重试前重新写入。
    ///
    /// X11 的 selection 所有权可能被其他客户端抢走（Xvfb 下也偶发），
    /// 只重复读是拿不回内容的；而"立即读"在写入方事件循环还没处理完时也可能读到旧值。
    /// 返回：`None` = 环境不支持写（不能判失败）；`Some(got)` = 最后一次读到的内容。
    fn set_then_get(c: &mut WidgetClipboard, text: &str) -> Option<Option<String>> {
        for attempt in 0..5 {
            c.set(text);
            std::thread::sleep(std::time::Duration::from_millis(40));
            let got = c.get();
            if got.as_deref() == Some(text) {
                return Some(got);
            }
            if attempt == 4 {
                return Some(got);
            }
        }
        None
    }

    #[test]
    fn adapter_forwards_to_system_clipboard() {
        let _guard = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut c = WidgetClipboard::system();
        let text = "adapter 测试 ABC 中文";
        // 读不回来只能说明环境不支持（无 X 会话 / 被外部客户端占用），不能当作失败
        match set_then_get(&mut c, text) {
            None => eprintln!("[跳过] 本环境剪贴板不可写"),
            Some(None) => eprintln!("[跳过] 本环境剪贴板读回为空（外部客户端接管）"),
            Some(Some(got)) => assert_eq!(got, text, "适配器应原样转发"),
        }
    }

    /// 进程内假剪贴板：让"适配器语义"可以被**确定性**断言，
    /// 不受 Xvfb / Windows 剪贴板全局状态影响。
    #[derive(Default)]
    struct FakeClipboard {
        text: std::sync::Mutex<String>,
    }

    impl crate::clipboard::Clipboard for FakeClipboard {
        fn get_text(&self) -> Result<String, String> {
            Ok(self.text.lock().unwrap_or_else(|e| e.into_inner()).clone())
        }
        fn set_text(&self, text: &str) -> Result<(), String> {
            *self.text.lock().unwrap_or_else(|e| e.into_inner()) = text.to_string();
            Ok(())
        }
    }

    #[test]
    fn empty_clipboard_maps_to_none() {
        // **确定性版本**：直接注入假剪贴板，断言适配器的映射规则。
        // 旧版本走系统剪贴板并 match 掉所有分支 —— 那等于没有断言。
        let fake = std::sync::Arc::new(FakeClipboard::default());
        let mut c = WidgetClipboard { inner: fake };
        c.set("");
        assert_eq!(
            c.get(),
            None,
            "空串必须映射成 None（控件视角没有可粘贴内容）"
        );

        c.set("x");
        assert_eq!(c.get().as_deref(), Some("x"), "非空内容必须原样返回");
    }

    #[test]
    fn unavailable_clipboard_surfaces_as_none() {
        // 读失败（环境不支持）在控件视角同样应是 None，而不是 panic / 空串。
        struct Broken;
        impl crate::clipboard::Clipboard for Broken {
            fn get_text(&self) -> Result<String, String> {
                Err("unavailable".to_string())
            }
            fn set_text(&self, _: &str) -> Result<(), String> {
                Err("unavailable".to_string())
            }
        }
        let mut c = WidgetClipboard {
            inner: std::sync::Arc::new(Broken),
        };
        assert_eq!(c.get(), None);
        c.set("ignored"); // 写失败被吞掉：Ctrl+V/Ctrl+C 失败要安静
        assert_eq!(c.get(), None);
    }
}
