//! 字体字形替身映射。
//!
//! 当字体缺少某个字符时，映射到一个替代字符。当前只处理**已确认缺失**的字符
//! （默认字体 NSimSun 缺 U+2212 数学减号），不做完整字体回退链。
//!
//! 完整字体回退（NSimSun → Segoe UI → YaHei）记为 P2 架构债，见 `docs/STATUS.md`。

/// 尝试为缺失字符找一个替身。
///
/// 返回 `Some(substitute)` 表示可以用替身字符渲染；`None` 表示没有替身。
///
/// 当前映射只有一条：
/// - `U+2212`（数学减号 MINUS SIGN）→ `U+002D`（ASCII 连字符），
///   在等宽布局下两者宽度一致，替换后不会改变单元格数。
///
/// **不扩展**：弯引号、省略号、乘号等经实测 NSimSun 都有，加映射只会掩盖真实缺字。
pub fn substitute(ch: char) -> Option<char> {
    match ch {
        '\u{2212}' => Some('-'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minus_sign_maps_to_ascii_hyphen() {
        assert_eq!(substitute('\u{2212}'), Some('-'));
    }

    #[test]
    fn ascii_hyphen_is_not_remapped() {
        // ASCII '-' 本身存在，不需要映射（否则会无谓地多一次查找）
        assert_eq!(substitute('-'), None);
    }

    #[test]
    fn other_characters_are_untouched() {
        assert_eq!(substitute('\u{4e2d}'), None);
        assert_eq!(substitute('A'), None);
        assert_eq!(substitute('\u{2018}'), None); // 弯引号：NSimSun 有，不映射
    }
}
