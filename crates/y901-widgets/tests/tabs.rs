//! Tabs 的交互、只渲染激活页、以及切换过渡的像素验收。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Modifiers, MouseButton, Pos, Size, UiEvent, UiTree};
use y901_widgets::tabs::Tabs;

const W: u32 = 240;
const H: u32 = 160;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

/// painter 调用记录（证明"只渲染激活页"）。
type Calls = Rc<RefCell<Vec<usize>>>;

/// 建 Tabs，返回控件、选中索引句柄与调用记录。
fn build() -> (Tabs, Rc<Cell<usize>>, Calls) {
    let calls: Calls = Rc::default();
    let c = calls.clone();
    let tabs = Tabs::new(
        "tabs",
        vec!["one".into(), "two".into(), "three".into()],
        Box::new(move |index, ctx, rect, progress| {
            c.borrow_mut().push(index);
            // 内容区涂白，alpha 随 progress 上升（模拟淡入）
            let a = progress.clamp(0.0, 1.0);
            ctx.fill_rect(rect, Color::rgba(1.0, 1.0, 1.0, a));
        }),
    );
    let handle = tabs.selection();
    (tabs, handle, calls)
}

fn key(k: y901_ui::Key) -> UiEvent {
    UiEvent::Key {
        key: k,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn click_selects_the_tab_under_the_pointer() {
    let (tabs, handle, _calls) = build();
    let mut tree = UiTree::new(Box::new(tabs));
    let soft = soft();
    tree.layout(Size::new(W as f32, H as f32), &soft);

    // 等分布局：三个标签各占 W/3，第二个标签中心在 W/3*1.5
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(W as f32 / 3.0 * 1.5, 16.0),
        button: MouseButton::Left,
    });
    assert_eq!(handle.get(), 1, "点击第二个标签后应选中索引 1");
}

#[test]
fn arrow_keys_cycle_selection_when_focused() {
    let (tabs, handle, _calls) = build();
    let mut tree = UiTree::new(Box::new(tabs));
    let soft = soft();
    tree.layout(Size::new(W as f32, H as f32), &soft);

    // 先点一下第一个标签拿到焦点（焦点路径走 request_focus，与真实交互一致）
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 16.0),
        button: MouseButton::Left,
    });
    tree.dispatch(&key(y901_ui::Key::Right));
    assert_eq!(handle.get(), 1, "右键应前进一页");
    tree.dispatch(&key(y901_ui::Key::Left));
    assert_eq!(handle.get(), 0, "左键应后退一页");
    tree.dispatch(&key(y901_ui::Key::Left));
    assert_eq!(handle.get(), 2, "索引 0 再左移应回绕到最后一页");
}

#[test]
fn only_the_active_page_is_painted() {
    let (tabs, handle, calls) = build();
    let mut tree = UiTree::new(Box::new(tabs));
    let mut soft = soft();
    tree.layout(Size::new(W as f32, H as f32), &soft);

    tree.invalidate_all();
    tree.render(&mut soft);
    assert_eq!(*calls.borrow(), vec![0], "初始只应绘制第 0 页");

    calls.borrow_mut().clear();
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 16.0),
        button: MouseButton::Left,
    });
    tree.dispatch(&key(y901_ui::Key::Right));
    tree.dispatch(&key(y901_ui::Key::Right));
    assert_eq!(handle.get(), 2);
    tree.invalidate_all();
    tree.render(&mut soft);
    assert_eq!(*calls.borrow(), vec![2], "切到第 2 页后只应绘制第 2 页");
}

#[test]
fn switch_transition_is_visible_in_pixels() {
    let (tabs, _handle, _calls) = build();
    let mut tree = UiTree::new(Box::new(tabs));
    let mut soft = soft();
    tree.layout(Size::new(W as f32, H as f32), &soft);

    // 起始帧：静止状态（progress = 1），内容全亮
    tree.invalidate_all();
    tree.render(&mut soft);

    // 切换后立刻渲染 = 过渡起点（内容被淡入遮罩压暗）
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 16.0),
        button: MouseButton::Left,
    });
    tree.dispatch(&key(y901_ui::Key::Right));
    tree.invalidate_all();
    soft.clear();
    tree.render(&mut soft);
    let dim = bright_pixels(&soft);

    // 推进过渡：每次 Tick 按真实时间推进，多打几次确保走完
    for _ in 0..10 {
        tree.dispatch(&UiEvent::Tick);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    tree.invalidate_all();
    soft.clear();
    tree.render(&mut soft);
    let full = bright_pixels(&soft);

    eprintln!("[tabs] 过渡起点亮像素={dim}，终态亮像素={full}");
    assert!(
        dim < full,
        "过渡中间帧应比终态更暗（{dim} vs {full}），否则说明动画没生效"
    );
}

/// 亮像素数量（内容区白色面积）。
fn bright_pixels(s: &SoftBackend) -> usize {
    let mut n = 0;
    for y in 0..H {
        for x in 0..W {
            let p = s.pixel(x, y).expect("像素");
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            if r > 200 && g > 200 && b > 200 {
                n += 1;
            }
        }
    }
    n
}
