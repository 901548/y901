//! 结构测试：`Options` 的 theme / font 是否真的接到了渲染后端。
//!
//! **这不是行为验证**：真正的行为（窗口变亮、字号变大）需要 GPU 设备，
//! 当前开发机的虚拟显卡拒绝 D3D11 设备创建，无法验证。
//! 真机验证清单见 `docs/FRAMEWORK_GUIDE.md` §27。
//!
//! 这里只保证"接线没被拆掉"：一旦有人删掉 `options.theme` 或
//! `TextEngine::new(&font.family, ...)`，测试立刻红。

#[test]
fn options_theme_reaches_backend() {
    let src = include_str!("../src/window.rs");

    assert!(
        src.contains("options.theme"),
        "Window::create 必须读取 options.theme（否则主题注入断链）"
    );
    assert!(
        src.contains("D3dBackend::new(hwnd, viewport.0, viewport.1, theme, &font)"),
        "D3dBackend::new 的调用点必须把 theme/font 传进去"
    );
}

#[test]
fn options_font_reaches_text_engine() {
    let src = include_str!("../src/backend/mod.rs");

    assert!(
        src.contains("font: &y901_runtime::FontConfig"),
        "D3dBackend::new 必须接收 FontConfig 参数"
    );
    assert!(
        src.contains("TextEngine::new(&font.family, font.size"),
        "TextEngine::new 必须拿到 family 与 size（不能退回硬编码常量）"
    );
    assert!(
        src.contains("cell_metrics_for(font.size)"),
        "cell 尺寸必须由字号推出，而不是写死 8×16"
    );
}
