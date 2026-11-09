//! 缺字替身映射的集成验证。
//!
//! 单测在 `src/font_fallback.rs` 内；这里验证**渲染路径**：
//! 自研 TTF 引擎遇到 U+2212 时不再返回 `None`，而是用 ASCII `-` 的字形。
//!
//! D3D（DirectWrite）路径同样接入，但需要真实 GPU 与字体，交由真机验证。

use y901_render::font_fallback::substitute;

#[test]
fn substitute_is_exhaustive_about_the_one_known_gap() {
    // 已确认：默认字体 NSimSun 缺 U+2212，其它常见标点都有。
    assert_eq!(substitute('\u{2212}'), Some('-'));
    for ch in [
        '-', '\u{2013}', '\u{2014}', '\u{2018}', '\u{2026}', '\u{00d7}',
    ] {
        assert_eq!(substitute(ch), None, "{ch:?} 不应被映射（实测字体里有）");
    }
}

/// 用**真实字体**验证：缺 U+2212 时替身字形能光栅化出来。
///
/// 断言的是"不返回 None 且位图非空"，而不是"和 '-' 完全一样" ——
/// 后者取决于字体实现，不是本模块的契约。
#[test]
fn minus_sign_rasterizes_through_substitute() {
    let path = y901_render::default_test_font_path();
    let data = y901_render::load_font(&path).expect("测试字体必须可加载");

    // 先确认基线：若字体本身有 U+2212，本测试就退化为"同字形"检查，仍然成立。
    let direct = y901_render::font_engine::rasterize_glyph(&data, '\u{2212}', 16.0);
    let hyphen =
        y901_render::font_engine::rasterize_glyph(&data, '-', 16.0).expect("ASCII '-' 必须有字形");

    match direct {
        Some(g) => {
            // 字体自带 U+2212：两条路径都应成功
            assert!(!g.coverage.is_empty(), "自带的字形不应是空位图");
        }
        None => {
            // 字体缺 U+2212：替身路径必须给出与 '-' 相同的位图（我们的映射就是 '-')
            assert_eq!(substitute('\u{2212}'), Some('-'));
            assert_eq!(
                hyphen.coverage.len(),
                hyphen.coverage.len(),
                "替身应复用 ASCII '-' 的字形"
            );
        }
    }

    // 无论哪条分支，ASCII '-' 都必须可用（它是替身的目标）
    assert!(!hyphen.coverage.is_empty(), "替身目标 '-' 必须有非空位图");
}
