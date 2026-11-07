//! `RenderCtx` 的**实现一致性**检查：D3D 后端必须提供与 CPU 后端同源的
//! `animation` / `set_animations` / `background_color`。
//!
//! # 为什么是结构断言而不是运行期对拍
//!
//! 运行期对拍需要**两个真实后端**：`D3dBackend` 必须由 `D3dBackend::new(hwnd, ..)` 构造，
//! 而 `hwnd` 只能来自真实窗口（`Window::create` 会建 HWND + D3D11 设备）。
//! 在没有 GPU / 没有窗口服务器的 CI 上，这条路径根本走不到。
//!
//! 于是这里改为钉住**实现事实**：两个后端的对应方法体里用的是同一个表达式
//! （`self.anims.get(key).unwrap_or(1.0)` 与 `self.theme.style(Role::PANEL, State::Normal).bg`）。
//! 之前 P1a/P1b 就是"方法根本不存在"导致的视觉 bug，结构断言正好能挡住这类回归。

#[test]
fn d3d_and_soft_share_the_same_animation_semantics() {
    let d3d = include_str!("../src/backend/mod.rs");
    let soft = include_str!("../../y901-render/src/soft.rs");

    // 两个后端都必须有这两个方法。
    for (name, src) in [("d3d", d3d), ("soft", soft)] {
        assert!(
            src.contains("fn animation(&self, key: u64) -> f32"),
            "{name} 缺少 RenderCtx::animation —— Popup 动画会恒为 1.0"
        );
        assert!(
            src.contains(
                "fn set_animations(&mut self, snapshot: &y901_ui::animation::AnimSnapshot)"
            ),
            "{name} 缺少 RenderCtx::set_animations —— 动画快照无法推入"
        );
    }

    // 语义必须同源：键不存在时回退 1.0。
    assert!(
        d3d.contains("self.anims.get(key).unwrap_or(1.0)"),
        "d3d 的 animation 必须与 SoftBackend 一样回退到 1.0"
    );
    assert!(
        soft.contains("self.anims.get(key).unwrap_or(1.0)"),
        "soft 的 animation 语义发生变化（本条断言用于对拍）"
    );
}

#[test]
fn d3d_and_soft_share_the_same_background_color_semantics() {
    let d3d = include_str!("../src/backend/mod.rs");
    let soft = include_str!("../../y901-render/src/soft.rs");

    for (name, src) in [("d3d", d3d), ("soft", soft)] {
        assert!(
            src.contains("fn background_color(&self) -> Color"),
            "{name} 缺少 RenderCtx::background_color —— Tabs 过渡遮罩会退化成黑色"
        );
        assert!(
            src.contains(".style(y901_ui::Role::PANEL, y901_ui::theme::State::Normal)"),
            "{name} 的 background_color 必须取面板背景色，而不是别的角色"
        );
    }
}

/// D3D 的 `stroke_round_rect` 必须与 `fill_round_rect` 一样处理 transform。
///
/// 为何钉死这三个字符串：旧版只调了 `clip_of`，
/// 于是 `map_rect`（位置/尺寸）、`radius * scale`、`width * scale`
/// 三件事全部缺失，有 transform 时描边直接错位，而编译器不会报警。
/// 这条断言就是为了阻止"将来又把它写回简版"。
#[test]
fn d3d_stroke_round_rect_applies_transform() {
    let src = include_str!("../src/backend/mod.rs");
    let start = src
        .find("fn stroke_round_rect(")
        .expect("D3dBackend::stroke_round_rect 未找到");
    let body = &src[start..];
    let end = body.find("fn stroke_rect(").unwrap_or(body.len());
    let body = &body[..end];

    assert!(
        body.contains("self.map_rect(rect)"),
        "stroke_round_rect 必须先 map_rect，否则有 transform 时描边错位"
    );
    assert!(
        body.contains("self.transforms.is_empty()"),
        "stroke_round_rect 必须判断是否有 transform（与 fill_round_rect 一致）"
    );
    assert!(
        body.contains("self.current_scale()"),
        "stroke_round_rect 必须把 radius 与 width 乘上 current_scale()"
    );
    assert!(
        body.contains("radius * s") && body.contains("width * s"),
        "radius 与 width 都要跟随缩放；只缩一个会让描边变形"
    );
}
