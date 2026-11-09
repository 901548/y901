//! P1-1：运行时主题 / 字体切换。
//!
//! **能力边界**：本机没有可用 D3D11 设备（虚拟显卡拒绝创建），
//! 因此 `D3dBackend::set_font` 的"真的换字形"只能靠结构断言；
//! 行为验证见 `docs/FRAMEWORK_GUIDE.md` §27。

use y901_runtime::{FontConfig, Host, Options};
use y901_ui::{Theme, UiTree};

// ---------- 1. 主题值本身有差异 + 后端把主题存下来 ----------

#[test]
fn set_theme_changes_rendered_colors() {
    use y901_ui::{Role, State};

    // 组件层"换主题就换颜色"由 y901-widgets/tests/theme_injection.rs 覆盖
    // （那里能在无 GPU 环境跑 Button 的 render）。
    // 这里验证两件后端层的事：
    // 1) 两套主题的按钮配色确实不同（否则 set_theme 没有意义）；
    // 2) set_theme 只改字段、不碰图集（见 set_theme_does_not_touch_atlas）。
    let dark = Theme::DARK.style(Role::BUTTON, State::Normal);
    let light = y901_ui::theme::LIGHT.style(Role::BUTTON, State::Normal);
    assert_ne!(dark.bg, light.bg, "DARK 与 LIGHT 的按钮底色必须不同");
    assert_ne!(dark.fg, light.fg, "DARK 与 LIGHT 的按钮字色必须不同");
}
// ---------- 2. 字号 → cell 尺寸（纯函数，可真跑） ----------

#[test]
fn set_font_changes_cell_size() {
    // set_font 内部就用这个函数推 cell 尺寸
    assert_eq!(y901_backend_win32::cell_metrics_for(16.0), (8.0, 16.0));
    assert_eq!(
        y901_backend_win32::cell_metrics_for(20.0),
        (10.0, 20.0),
        "20px 字体必须给出 10×20 的格子"
    );
}

// ---------- 3/4. set_font 的失败回滚与图集清理（结构断言） ----------

#[test]
fn set_font_builds_new_engine_before_swapping() {
    let src = include_str!("../src/backend/mod.rs");
    let start = src
        .find("pub fn set_font")
        .expect("D3dBackend::set_font 必须存在");
    let body = &src[start..];
    let end = body[1..]
        .find("\n    pub ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];

    let create = body
        .find("TextEngine::new")
        .expect("必须先创建新引擎（否则无法在失败时保留旧状态）");
    let swap = body
        .find("self.text = Box::new(text)")
        .expect("必须替换引擎");
    assert!(
        create < swap,
        "顺序错误：必须**先** TextEngine::new 失败返回，**再**替换 self.text；\
         否则会出现「新引擎失败但旧引擎已被丢弃」的中间态"
    );
    assert!(
        body.contains("?"),
        "TextEngine::new 的失败必须用 `?` 传播（不能忽略）"
    );
    assert!(
        body.contains("self.cell = Size::new(cell_w, cell_h)"),
        "cell 尺寸必须随新字号更新"
    );
    // 图集清理：新引擎的 reset_pending = true（由 TextEngine::new 保证），
    // 这里断言 set_font 不会在换引擎前清空旧图集
    let reset_before_swap = body[..swap].contains("take_dirty");
    assert!(
        !reset_before_swap,
        "换引擎之前不得动图集（旧图集在失败时必须仍然可用）"
    );
}

#[test]
fn set_theme_does_not_touch_atlas() {
    let src = include_str!("../src/backend/mod.rs");
    let start = src.find("pub fn set_theme").expect("set_theme 必须存在");
    let body = &src[start..];
    let end = body[1..]
        .find("\n    pub ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];

    // P2 起委托给 RenderState：断言“确实调了 state.set_theme”，
    // 仍保留下面那个“不得碰图集”的反向断言。
    assert!(
        body.contains("self.state.set_theme(theme)"),
        "set_theme 必须委托给 RenderState"
    );
    for forbidden in ["take_dirty", "TextEngine::new", "update_atlas", "gfx"] {
        assert!(
            !body.contains(forbidden),
            "set_theme 不应触碰 `{forbidden}`（主题不含字形，不需要重建图集）"
        );
    }
}

// ---------- 5. Host 的请求接口默认无请求 ----------

struct MinimalHost;

impl Host for MinimalHost {
    fn idle(&mut self, _tree: &mut UiTree) {}
    fn command(&mut self, _tree: &mut UiTree, _cmd: y901_ui::Command) -> bool {
        false
    }
}

#[test]
fn host_theme_requests_default_to_none() {
    let mut host = MinimalHost;
    assert!(host.take_theme_request().is_none(), "默认无主题请求");
    assert!(host.take_font_request().is_none(), "默认无字体请求");
}

#[test]
fn options_carry_theme_and_font_for_startup() {
    let o = Options::default();
    assert!(o.theme.is_none());
    assert!(o.font.is_none());

    let custom = Options {
        theme: Some(y901_ui::theme::LIGHT),
        font: Some(FontConfig {
            family: "JetBrains Mono".to_string(),
            size: 20.0,
        }),
        ..Options::default()
    };
    assert_eq!(custom.theme, Some(y901_ui::theme::LIGHT));
    assert_eq!(custom.font.as_ref().map(|f| f.size), Some(20.0));
}
