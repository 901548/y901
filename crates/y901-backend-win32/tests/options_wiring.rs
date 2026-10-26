//! 结构测试：`Options` 的 theme / font 是否真的接到了渲染后端。
//!
//! **这不是行为验证**：真正的行为（窗口变亮、字号变大）需要 GPU 设备，
//! 当前开发机的虚拟显卡拒绝 D3D11 设备创建，无法验证。
//! 真机验证清单见 `docs/FRAMEWORK_GUIDE.md` §27。
//!
//! 断言方式：先**切出函数体**，再在函数体内找片段。
//! 这样 rustfmt 拆行、加注释、改变量名都不会假红；
//! 只要关键调用还在该函数里，测试就过。

/// 从 `src` 里切出以 `signature` 开头的函数体。
///
/// 输入：源码、函数签名前缀（例如 `"unsafe fn create_inner"`）。
/// 返回：从签名起到下一个同级成员（4 空格缩进的 `pub`/`fn`/`unsafe`/`const`）
/// 或文件末尾之间的切片。
/// 失败：找不到签名时 panic（说明函数被改名/删除——这本身就该红）。
fn body_of<'a>(src: &'a str, signature: &str) -> &'a str {
    let start = src
        .find(signature)
        .unwrap_or_else(|| panic!("源码里找不到 `{signature}`（函数被改名或删除）"));
    let rest = &src[start..];
    let mut end = rest.len();
    for needle in ["\n    pub ", "\n    fn ", "\n    unsafe fn ", "\n    const "] {
        if let Some(i) = rest[1..].find(needle) {
            // +1 因为检索从第 2 个字节开始
            let abs = i + 1;
            if abs < end {
                end = abs;
            }
        }
    }
    &rest[..end]
}

#[test]
fn options_theme_reaches_backend() {
    let src = include_str!("../src/window.rs");
    let body = body_of(src, "unsafe fn create_inner");

    assert!(
        body.contains("options.theme"),
        "Window::create 必须读取 options.theme（否则主题注入断链）"
    );
    assert!(
        body.contains("D3dBackend::new"),
        "create_inner 里必须构造 D3dBackend"
    );
    assert!(
        body.contains("theme"),
        "D3dBackend::new 的实参里必须有 theme"
    );
    assert!(
        body.contains("&font"),
        "D3dBackend::new 的实参里必须有 &font"
    );
    // 切片守卫：函数体不能溢出到后面的函数（否则上面的断言会假阳性）
    assert!(
        !body.contains("WM_KEYDOWN"),
        "切片越界：create_inner 的体里不该出现后面的 wndproc 内容"
    );
}

#[test]
fn options_font_reaches_text_engine() {
    let src = include_str!("../src/backend/mod.rs");
    let body = body_of(src, "pub unsafe fn new(");

    assert!(
        body.contains("FontConfig"),
        "D3dBackend::new 必须接收 FontConfig 参数"
    );
    assert!(
        body.contains("TextEngine::new"),
        "必须构造 TextEngine"
    );
    assert!(
        body.contains("font.family") && body.contains("font.size"),
        "TextEngine::new 必须拿到 family 与 size（不能退回硬编码常量）"
    );
    // 断言**调用形式**而不是标识符：注释里出现 `cell_metrics_for` 不算接线
    assert!(
        body.contains("cell_metrics_for(font.size)"),
        "cell 尺寸必须由字号推出（调用 cell_metrics_for(font.size)），而不是写死 8×16"
    );
    // 切片守卫：函数体不能溢出到后面的方法（`rebuild` 在 `new` 之后）
    assert!(
        !body.contains("fn rebuild"),
        "切片越界：D3dBackend::new 的体里不该出现 rebuild"
    );
}
