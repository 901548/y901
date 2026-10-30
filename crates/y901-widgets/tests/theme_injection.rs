//! P0-2：主题可注入。
//!
//! 渲染上下文决定颜色：`ctx.theme()` 返回哪个主题，组件就用哪套配色。
//! 生产路径由 `Options.theme` → `D3dBackend.theme` 注入；这里验证"注入即变色"。

use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle, Theme, UiTree, Widget};
use y901_widgets::Button;

/// 记录所有 `fill_rect` 颜色的上下文；主题可注入。
struct Recorder {
    theme: Theme,
    fills: Vec<Color>,
}

impl TextMetrics for Recorder {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Recorder {
    fn size(&self) -> Size {
        Size::new(200.0, 100.0)
    }
    fn fill_rect(&mut self, _r: Rect, color: Color) {
        self.fills.push(color);
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, t: &str, s: &TextStyle) -> f32 {
        self.measure_text(t, s).w
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
    fn theme(&self) -> &Theme {
        &self.theme
    }
}

fn render_button(theme: Theme) -> Vec<Color> {
    let mut btn = Button::new("btn", "OK");
    btn.arrange(
        Rect::new(0.0, 0.0, 100.0, 40.0),
        &Recorder {
            theme,
            fills: Vec::new(),
        },
        &mut y901_ui::layout::LayoutCache::new(),
    );
    let mut ctx = Recorder {
        theme,
        fills: Vec::new(),
    };
    btn.render(&mut ctx);
    ctx.fills
}

#[test]
fn theme_injection_changes_button_color() {
    let dark = render_button(Theme::DARK);
    let light = render_button(y901_ui::theme::LIGHT);

    assert!(!dark.is_empty(), "按钮必须至少画一个矩形");
    assert_ne!(dark, light, "注入 LIGHT 主题后，按钮的颜色必须与 DARK 不同");
}

#[test]
fn default_theme_is_dark() {
    // 不覆写 theme() 的上下文必须拿到 DARK_THEME（框架默认）
    struct DefaultTheme;
    impl TextMetrics for DefaultTheme {
        fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
            Size::new(8.0, 16.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            16.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }
    impl RenderCtx for DefaultTheme {
        fn size(&self) -> Size {
            Size::ZERO
        }
        fn fill_rect(&mut self, _r: Rect, _c: Color) {}
        fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
        fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
            0.0
        }
        fn push_clip(&mut self, _r: Rect) {}
        fn pop_clip(&mut self) {}
    }

    let ctx = DefaultTheme;
    assert_eq!(
        ctx.theme(),
        &y901_ui::theme::DARK_THEME,
        "未注入主题时必须是暗色"
    );
}

#[test]
fn rendering_a_tree_does_not_require_a_theme() {
    // 回归：不做任何主题注入时，整棵树仍能渲染（用默认主题）。
    let mut tree = UiTree::new(Box::new(Button::new("btn", "OK")));
    tree.layout(
        Size::new(100.0, 40.0),
        &Recorder {
            theme: Theme::DARK,
            fills: Vec::new(),
        },
    );
    let mut ctx = Recorder {
        theme: Theme::DARK,
        fills: Vec::new(),
    };
    tree.render(&mut ctx);
    assert!(!ctx.fills.is_empty(), "默认主题下也要画出东西");
}
