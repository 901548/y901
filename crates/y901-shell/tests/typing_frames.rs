//! 端到端复现：在**演示页**上打字，逐帧检查脏区是否退化成整窗。
//!
//! 报障现象是"每敲一个字符引起 3~4 次整屏重绘"。框架层单测（`typing_dirty_scope.rs`）
//! 已证明 `TextInput` 的 `Relayout` 只标目标矩形；本测试更接近真机：
//! 用**同一份演示页**（`build_demo_page`，桌面与 wasm 共用）+ 真正的 `frame_step`
//! 主循环 + 记录型 `WindowBackend`，把"按键 → 布局 → 渲染"整条链路跑一遍，
//! 断言每一帧的脏区都**不是** `full`。

use std::collections::VecDeque;
use std::time::Duration;

use y901_runtime::runtime::frame_step;
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{Host, RawEvent};
use y901_shell::build_demo_page;
use y901_ui::{
    Color, Command, Key, Modifiers, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle, UiTree,
};

const VW: u32 = 800;
const VH: u32 = 600;

/// 与浏览器初始格子一致的度量（8×16，CJK 双宽）。
struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, style: &TextStyle) -> Size {
        let px = style.size.unwrap_or(16.0);
        let per = if (px - 16.0).abs() < 0.01 {
            8.0
        } else {
            px * 0.6
        };
        let mut w = 0.0;
        for ch in text.chars() {
            w += if (ch as u32) >= 0x2E80 {
                per * 2.0
            } else {
                per
            };
        }
        Size::new(w, px.max(12.0))
    }
    fn line_height(&self, style: &TextStyle) -> f32 {
        style.size.unwrap_or(16.0).max(12.0)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 什么都不画：只让 `UiTree::render` 能正常清空脏区。
struct Null;

impl TextMetrics for Null {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Mono.measure_text(t, s)
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        Mono.line_height(s)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Null {
    fn size(&self) -> Size {
        Size::new(VW as f32, VH as f32)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

/// 一帧的观测：脏区是否整窗、脏矩形列表。
#[derive(Debug, Clone)]
struct Frame {
    full: bool,
    rects: Vec<Rect>,
}

struct Recording {
    pending: VecDeque<RawEvent>,
    frames: Vec<Frame>,
    layouts: usize,
}

impl Recording {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            frames: Vec::new(),
            layouts: 0,
        }
    }

    fn push(&mut self, ev: RawEvent) {
        self.pending.push_back(ev);
    }
}

impl WindowBackend for Recording {
    fn wait_events(&mut self, _wait_ms: u32) -> Option<Vec<RawEvent>> {
        Some(self.pending.drain(..).collect())
    }
    fn viewport(&self) -> (u32, u32) {
        (VW, VH)
    }
    fn relayout(&mut self, tree: &mut UiTree) {
        self.layouts += 1;
        tree.layout(Size::new(VW as f32, VH as f32), &Mono);
    }
    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String> {
        if tree.dirty().is_empty() {
            return Ok(RenderOutcome::Skipped);
        }
        self.frames.push(Frame {
            full: tree.dirty().is_full(),
            rects: tree.dirty().rects().to_vec(),
        });
        tree.render(&mut Null);
        Ok(RenderOutcome::Presented)
    }
    fn set_theme(&mut self, _theme: y901_ui::theme::Theme) -> Result<(), String> {
        Ok(())
    }
}

struct QuietHost;

impl Host for QuietHost {
    fn idle(&mut self, _tree: &mut UiTree) {}
    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false
    }
}

fn setup() -> (Recording, UiTree, QuietHost) {
    let mut tree = build_demo_page();
    tree.layout(Size::new(VW as f32, VH as f32), &Mono);
    tree.set_focus(Some("user-in".to_string()));
    let mut window = Recording::new();
    let mut host = QuietHost;
    // 先空跑两帧，把启动时的整窗标脏消化掉（真实浏览器启动也只整窗重绘一次）。
    for _ in 0..2 {
        step(&mut window, &mut tree, &mut host);
    }
    window.frames.clear();
    (window, tree, host)
}

fn step(window: &mut Recording, tree: &mut UiTree, host: &mut QuietHost) {
    // 与真实主循环一致：先取事件，再把事件交给 `frame_step`。
    let evs = window.wait_events(0).unwrap_or_default();
    let _ = frame_step(window, tree, host, &evs, Duration::from_millis(16));
}

fn run_frames(window: &mut Recording, tree: &mut UiTree, host: &mut QuietHost, n: usize) {
    for _ in 0..n {
        step(window, tree, host);
    }
}

/// 一次物理按键（KeyDown + Char，与浏览器/桌面的成对事件一致）只应产生**局部**脏区。
#[test]
fn demo_page_keystroke_never_dirties_whole_window() {
    let (mut window, mut tree, mut host) = setup();
    assert_eq!(tree.focus(), Some("user-in"), "前置：输入框获焦");
    let input = tree.rect_of("user-in").expect("user input rect");
    assert!(input.w > 0.0 && input.h > 0.0, "前置：输入框已布局");

    for ch in ['a', 'b', 'c'] {
        window.push(RawEvent::KeyDown {
            key: Key::Char(ch),
            mods: Modifiers::default(),
        });
        window.push(RawEvent::Char { c: ch });
        run_frames(&mut window, &mut tree, &mut host, 1);
    }

    assert!(!window.frames.is_empty(), "按键必须至少出过一帧");
    for (i, frame) in window.frames.iter().enumerate() {
        assert!(
            !frame.full,
            "第 {i} 帧是整窗重绘（脏区 full）：{:?}",
            frame.rects
        );
        for r in &frame.rects {
            assert!(
                r.w <= input.w + 2.0 && r.h <= input.h + 2.0,
                "第 {i} 帧的脏区 {r:?} 比输入框 {input:?} 还大"
            );
        }
    }
}

/// 连续打字：每帧都局部，且不会因为碎片累积（> 16 块）降级为整窗。
#[test]
fn demo_page_typing_stays_partial_over_many_frames() {
    let (mut window, mut tree, mut host) = setup();

    for (i, ch) in "abcdefghijklmnopqrst".chars().enumerate() {
        window.push(RawEvent::KeyDown {
            key: Key::Char(ch),
            mods: Modifiers::default(),
        });
        window.push(RawEvent::Char { c: ch });
        run_frames(&mut window, &mut tree, &mut host, 1);
        let last = window.frames.last().expect("每帧都应出画面");
        assert!(!last.full, "第 {i} 次按键退化成整窗重绘：{:?}", last.rects);
    }

    let full_frames = window.frames.iter().filter(|f| f.full).count();
    assert_eq!(full_frames, 0, "打字过程中不应出现整窗重绘");
}

/// 焦点切换（点击进入输入框）后紧接打字：焦点帧允许较宽，
/// 但后续按键帧必须是局部。
#[test]
fn focus_frame_may_be_wide_but_keystrokes_are_local() {
    let (mut window, mut tree, mut host) = setup();
    tree.set_focus(None);
    window.push(RawEvent::MouseDown {
        x: 300.0,
        y: 120.0,
        button: y901_ui::MouseButton::Left,
    });
    window.push(RawEvent::MouseUp {
        x: 300.0,
        y: 120.0,
        button: y901_ui::MouseButton::Left,
    });
    run_frames(&mut window, &mut tree, &mut host, 1);

    let before = window.frames.len();
    for ch in ['x', 'y'] {
        window.push(RawEvent::KeyDown {
            key: Key::Char(ch),
            mods: Modifiers::default(),
        });
        window.push(RawEvent::Char { c: ch });
        run_frames(&mut window, &mut tree, &mut host, 1);
    }
    for (i, frame) in window.frames.iter().enumerate().skip(before) {
        assert!(!frame.full, "焦点后的第 {i} 帧不应整窗重绘");
    }
}
