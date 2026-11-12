//! `PlatformRuntime::run_many` 的**契约测试**（默认实现，纯逻辑，不需要窗口）。
//!
//! 契约（见 `PlatformRuntime::run_many` 的文档）：
//! 1. `make` 对每个窗口**恰好调用一次**（下标 `0..options_list.len()`）；
//! 2. 错误信息**带窗口序号**（格式 `窗口 {i}: {e}`）；
//! 3. 所有已启动的窗口最终都会被关闭（正常关闭或错误返回）。
//!
//! 这里测的是**默认实现**（串行、遇错即停）。平台实现（Win32 / X11）的并行/聚合语义
//! 属于"实现相关"的文档化差异，其契约由各自的 `#[ignore]` 测试或脚本验证。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use y901_runtime::{Host, Options, PlatformRuntime, WindowFactory};
use y901_ui::{
    layout::LayoutCache, Command, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics,
    Size, TextMetrics, UiEvent, UiTree, Widget,
};

// ---------------------------------------------------------------- 最小 mock

/// 什么都不做的 `Host`（本测试不关心回调）。
struct NoopHost;

impl Host for NoopHost {
    fn idle(&mut self, _tree: &mut UiTree) {}
    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false
    }
}

/// 最小 `Widget`：只提供 id/尺寸，不画、不响应事件。
struct NoopWidget {
    id: &'static str,
    rect: Rect,
}

impl NoopWidget {
    fn new() -> Self {
        Self {
            id: "root",
            rect: Rect::ZERO,
        }
    }
}

impl Widget for NoopWidget {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// mock 运行时：`run` 只计数（不建窗），可按序号注入失败。
struct MockRuntime {
    /// `run` 被调用的次数（= 已"跑过"的窗口数）。
    runs: Arc<AtomicUsize>,
    /// 第几个 `run` 调用返回 `Err`（`None` = 全部成功）。
    fail_at: Option<usize>,
}

impl PlatformRuntime for MockRuntime {
    fn run(
        &mut self,
        _host: Box<dyn Host>,
        _build: Box<dyn FnOnce() -> UiTree>,
        _options: Options,
    ) -> Result<(), String> {
        let n = self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fail_at == Some(n) {
            Err(format!("mock-boom-{n}"))
        } else {
            Ok(())
        }
    }
}

/// 造一个工厂：记录 `make` 调用次数。
fn counting_factory(calls: Arc<AtomicUsize>) -> WindowFactory {
    Box::new(move |_i| {
        calls.fetch_add(1, Ordering::SeqCst);
        (
            Box::new(NoopHost) as Box<dyn Host>,
            UiTree::new(Box::new(NoopWidget::new())),
        )
    })
}

// ---------------------------------------------------------------- 契约 1 + 3

#[test]
fn default_run_many_calls_factory_exactly_once_per_window() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = counting_factory(Arc::clone(&calls));

    let options = vec![Options::default(), Options::default(), Options::default()];
    let mut rt = MockRuntime {
        runs: Arc::new(AtomicUsize::new(0)),
        fail_at: None,
    };

    let result = rt.run_many(options, make);
    assert!(result.is_ok(), "默认实现在全部成功时应返 Ok：{result:?}");

    // **精确等于**（不是 `<=`）：少调或多调都是契约破坏。
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "`make` 必须对每个窗口恰好调用一次"
    );
    assert_eq!(
        rt.runs.load(Ordering::SeqCst),
        3,
        "`run` 必须对每个窗口恰好调用一次（所有已启动窗口都被跑完）"
    );
}

// ---------------------------------------------------------------- 契约 2

#[test]
fn default_run_many_error_carries_window_index() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = counting_factory(Arc::clone(&calls));

    let options = vec![Options::default(), Options::default()];
    let mut rt = MockRuntime {
        runs: Arc::new(AtomicUsize::new(0)),
        fail_at: Some(0),
    };

    let err = rt
        .run_many(options, make)
        .expect_err("第 0 个窗口失败时应返回 Err");

    assert!(err.contains("窗口 0"), "错误信息必须带窗口序号：{err}");
    assert!(
        err.contains("mock-boom-0"),
        "错误信息必须保留原始错误文本：{err}"
    );
}

// ---------------------------------------------------------------- 契约 1 的"短路"侧

/// 默认实现是**串行 + 短路**：第 0 个失败后，**第 1 个不会被启动**。
///
/// 这是"实现相关"的文档化语义（平台实现是并行 + 聚合）。
/// 把这条也测出来，是为了**防止有人把默认实现改成并行**而不改文档。
#[test]
fn default_run_many_short_circuits_after_first_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = counting_factory(Arc::clone(&calls));

    let options = vec![Options::default(), Options::default(), Options::default()];
    let mut rt = MockRuntime {
        runs: Arc::new(AtomicUsize::new(0)),
        fail_at: Some(0),
    };

    let err = rt.run_many(options, make).expect_err("应失败");
    assert!(err.contains("窗口 0"), "{err}");

    assert_eq!(
        rt.runs.load(Ordering::SeqCst),
        1,
        "默认实现应短路：失败后不再启动后续窗口"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "`make` 也不应对未启动的窗口调用"
    );
}

// ---------------------------------------------------------------- 契约 1：序号严格递增

/// 工厂记录它收到的**序号序列**（不只是次数）。
fn index_recording_factory(seen: Arc<std::sync::Mutex<Vec<usize>>>) -> WindowFactory {
    Box::new(move |i| {
        if let Ok(mut v) = seen.lock() {
            v.push(i);
        }
        (
            Box::new(NoopHost) as Box<dyn Host>,
            UiTree::new(Box::new(NoopWidget::new())),
        )
    })
}

/// **`make` 拿到的下标必须是 `0, 1, 2, …` 且严格递增**——
/// 契约只写"下标 `0..options_list.len()`"，若实现传错序号，
/// 多窗口错误信息就会指向错窗口。
#[test]
fn default_run_many_passes_sequential_indices() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let make = index_recording_factory(Arc::clone(&seen));

    let options = vec![Options::default(), Options::default(), Options::default()];
    let mut rt = MockRuntime {
        runs: Arc::new(AtomicUsize::new(0)),
        fail_at: None,
    };

    rt.run_many(options, make).expect("全成功应返 Ok");
    let got = seen.lock().unwrap().clone();
    assert_eq!(got, vec![0usize, 1, 2], "make 应依次收到 0,1,2");
}

// ---------------------------------------------------------------- 空列表

/// **空列表 = 不启动任何窗口，直接 `Ok(())`**。
///
/// 为何要测：这是"不做事"的边界；若实现在空列表上 panic 或返 `Err`，
/// 调用方就无法用"空配置"表达"不开窗口"。
#[test]
fn default_run_many_empty_list_is_ok_and_never_calls_factory() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = counting_factory(Arc::clone(&calls));
    let mut rt = MockRuntime {
        runs: Arc::new(AtomicUsize::new(0)),
        fail_at: None,
    };

    let result = rt.run_many(Vec::new(), make);
    assert!(result.is_ok(), "空列表应返 Ok：{result:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "空列表不应调用 make");
    assert_eq!(rt.runs.load(Ordering::SeqCst), 0, "空列表不应调用 run");
}

// ---------------------------------------------------------------- 契约 2：非 0 序号的错误

/// **第 1 个窗口失败时**，错误应指向窗口 1（不是 0），
/// 且第 0 个已跑完（`runs == 2`）、第 2 个未启动（`calls == 2`）。
///
/// 已有的 `error_carries_window_index` 只测 `fail_at = 0`——
/// 那种情况下"序号 0"恰好与"没有序号"难分，所以必须补一条非 0 序号。
#[test]
fn default_run_many_error_index_is_accurate_for_later_window() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = counting_factory(Arc::clone(&calls));
    let mut rt = MockRuntime {
        runs: Arc::new(AtomicUsize::new(0)),
        fail_at: Some(1),
    };

    let options = vec![
        Options::default(),
        Options::default(),
        Options::default(),
    ];
    let err = rt.run_many(options, make).expect_err("第 1 个窗口失败时应返 Err");
    assert!(err.contains("窗口 1"), "错误必须指向窗口 1：{err}");
    assert!(err.contains("mock-boom-1"), "应保留原始错误：{err}");
    assert_eq!(rt.runs.load(Ordering::SeqCst), 2, "第 0、1 个跑完后才失败");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "第 2 个窗口不应被启动");
}
