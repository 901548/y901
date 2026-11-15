//! DOM 事件 → [`y901_ui::Key`] / [`MouseButton`] / [`Modifiers`] 的**纯函数**映射。
//!
//! 这一层是整个 S2 里唯一能脱离浏览器验证的部分：不碰 DOM、不碰 wasm，
//! 因此每个分支都能用本机 `cargo test` 钉死。DOM 接线（P1 起）只负责把
//! 事件字段取出来喂给这些函数。
//!
//! **与桌面后端同构**：映射目标与 `y901-backend-win32` 的 `vk_to_key`
//! （`window.rs:245`）一致 —— 特殊键 → `Key::{Enter, Escape, …}`，
//! 可打印字符 → `Key::Char`，其余 → `None`（由调用方决定是否降级为
//! `Key::Unknown`，映射层不替它编造按键）。
//!
//! 三条约定，都写进了单测：
//!
//! 1. **特殊键按 `KeyboardEvent.code` 识别**，因为它与键盘布局无关；
//!    `code` 也对不上时才看 `key`。
//! 2. **可打印字符取自 `KeyboardEvent.key`**（已过布局/IME 归一），
//!    且必须是**单个 char**；控制字符与多字符（如 `"Dead"`、`"Unidentified"`）一律不算字符。
//! 3. **`Meta`（⌘/Win）不进 [`Modifiers`]**：框架的修饰键只有
//!    shift / ctrl / alt，映射层不擅自把 Meta 折叠成 Ctrl。

use y901_ui::{Key, Modifiers, MouseButton};

/// `KeyboardEvent.code` + `KeyboardEvent.key` → [`Key`]。
///
/// 返回 `None` 表示"这个键框架不认识"，调用方（P1 的监听器）应当**丢弃**
/// 而不是伪造一个 `Key::Char('\0')`。
pub fn key_from_dom(code: &str, key: &str) -> Option<Key> {
    // 1) 特殊键：优先按 code（布局无关）。
    if let Some(k) = special_key(code) {
        return Some(k);
    }
    // 2) 功能键 F1..F24：`code` 形如 "F12"。
    if let Some(n) = function_key(code) {
        return Some(Key::Function(n));
    }
    // 3) 可打印字符：取 key 的单字符形式（空格单独处理，避免 '\u{20}' 歧义）。
    if key == " " {
        return Some(Key::Char(' '));
    }
    if let Some(c) = single_printable_char(key) {
        return Some(Key::Char(c));
    }
    None
}

/// `KeyboardEvent.key` → 要作为 `RawEvent::Char` 提交的字符。
///
/// 与 [`key_from_dom`] 分开：一个按键既要产生 `KeyDown`（供快捷键/焦点用），
/// 可打印时还要产生 `Char`（供 `TextInput` 插入）。是否真的发 `Char`
/// 取决于修饰键（Ctrl/Alt 组合不该插入文本），那一步在调用方。
pub fn char_from_dom(key: &str) -> Option<char> {
    if key == " " {
        return Some(' ');
    }
    single_printable_char(key)
}

/// `MouseEvent.button` → [`MouseButton`]。
///
/// DOM 规范：`0` 主键（左）、`1` 中键、`2` 次键（右）、`3/4` 侧键。
/// 侧键框架没有对应语义，返回 `None`。
pub fn button_from_dom(button: i16) -> Option<MouseButton> {
    match button {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Middle),
        2 => Some(MouseButton::Right),
        _ => None,
    }
}

/// 四个 DOM 修饰键布尔 → [`Modifiers`]。
///
/// `meta` 被显式忽略：框架的 `Modifiers` 没有 meta 字段，
/// 把 ⌘ 当成 Ctrl 会让 macOS 上的普通复制变成别的语义。
pub fn mods_from_event(ctrl: bool, shift: bool, alt: bool, meta: bool) -> Modifiers {
    let _ = meta;
    Modifiers {
        ctrl,
        shift,
        alt,
    }
}

/// `WheelEvent.deltaY` → 框架的 `RawEvent::Wheel.delta`。
///
/// **符号相反**：DOM 里 `deltaY > 0` 表示向下滚，框架约定 `delta` 正数表示向上。
pub fn wheel_delta_from_dom(delta_y: f64) -> f32 {
    // 非有限值（部分浏览器在合成事件里给 NaN）按 0 处理，避免污染滚动累加。
    if delta_y.is_finite() {
        -delta_y as f32
    } else {
        0.0
    }
}

/// **DOM 像素坐标 → UI 树坐标**的 Y 转换（仅 Y）。
///
/// 为何要转：`SoftBackend` 交出的缓冲是**自下而上**存的，`present` 又把整帧按行翻转一次，
/// 两次叠加后**画面正向**，但 UI 树的 y 坐标仍然是"从屏幕底部往上"计的。
/// 而 DOM 的 `offsetY` 是"从屏幕顶部往下"。两者方向相反，不转换就会点上去下错位
/// （典型现象：点上面的输入框，焦点落在下面那个）。
///
/// 公式：`y_ui = height - 1 - y_dom`。`-1` 是为了让**最上一行像素**落在 `height - 1`，
/// 而非越界到 `height`。
pub fn to_ui_y(height: f32, y_dom: f32) -> f32 {
    // 非有限值不要污染命中测试：取 0（屏幕底部）作为安全值。
    if !height.is_finite() || !y_dom.is_finite() {
        return 0.0;
    }
    (height - 1.0 - y_dom).max(0.0)
}

/// 特殊键表：`(code, Key)`。
fn special_key(code: &str) -> Option<Key> {
    let k = match code {
        "Enter" | "NumpadEnter" => Key::Enter,
        "Escape" => Key::Escape,
        "Backspace" => Key::Backspace,
        "Tab" => Key::Tab,
        "Delete" => Key::Delete,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        _ => return None,
    };
    Some(k)
}

/// `"F1"`..`"F24"` → `Key::Function(n)`；越界或不匹配返回 `None`。
fn function_key(code: &str) -> Option<u8> {
    let digits = code.strip_prefix('F')?;
    // 只接受纯数字，避免 "F"、"Fox" 之类被误判。
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u8 = digits.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

/// 单个可打印字符；控制字符（含 `\t`、`\r`、`\u{7f}`）与多字符一律 `None`。
fn single_printable_char(key: &str) -> Option<char> {
    let mut it = key.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None;
    }
    if c.is_control() {
        return None;
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- key_from_dom：特殊键 ----

    #[test]
    fn maps_special_keys_by_code() {
        assert_eq!(key_from_dom("Enter", "Enter"), Some(Key::Enter));
        assert_eq!(key_from_dom("NumpadEnter", "Enter"), Some(Key::Enter));
        assert_eq!(key_from_dom("Escape", "Escape"), Some(Key::Escape));
        assert_eq!(key_from_dom("Backspace", "Backspace"), Some(Key::Backspace));
        assert_eq!(key_from_dom("Tab", "Tab"), Some(Key::Tab));
        assert_eq!(key_from_dom("Delete", "Delete"), Some(Key::Delete));
    }

    #[test]
    fn maps_arrow_keys_by_code() {
        assert_eq!(key_from_dom("ArrowUp", "ArrowUp"), Some(Key::Up));
        assert_eq!(key_from_dom("ArrowDown", "ArrowDown"), Some(Key::Down));
        assert_eq!(key_from_dom("ArrowLeft", "ArrowLeft"), Some(Key::Left));
        assert_eq!(key_from_dom("ArrowRight", "ArrowRight"), Some(Key::Right));
    }

    #[test]
    fn maps_navigation_keys_by_code() {
        assert_eq!(key_from_dom("Home", "Home"), Some(Key::Home));
        assert_eq!(key_from_dom("End", "End"), Some(Key::End));
        assert_eq!(key_from_dom("PageUp", "PageUp"), Some(Key::PageUp));
        assert_eq!(key_from_dom("PageDown", "PageDown"), Some(Key::PageDown));
    }

    // ---- key_from_dom：功能键 ----

    #[test]
    fn maps_function_keys_within_range() {
        assert_eq!(key_from_dom("F1", "F1"), Some(Key::Function(1)));
        assert_eq!(key_from_dom("F12", "F12"), Some(Key::Function(12)));
        assert_eq!(key_from_dom("F24", "F24"), Some(Key::Function(24)));
    }

    #[test]
    fn rejects_out_of_range_or_malformed_function_keys() {
        assert_eq!(key_from_dom("F0", "F0"), None);
        assert_eq!(key_from_dom("F25", "F25"), None);
        assert_eq!(key_from_dom("Fox", "Fox"), None);
        // 只有 code 是 "F"、key 是单字符时，它不是功能键而是字母 F
        assert_eq!(key_from_dom("KeyF", "F"), Some(Key::Char('F')));
    }

    // ---- key_from_dom：字符键 ----

    #[test]
    fn maps_printable_characters_from_key() {
        assert_eq!(key_from_dom("KeyA", "a"), Some(Key::Char('a')));
        assert_eq!(key_from_dom("Digit8", "8"), Some(Key::Char('8')));
        // 非 ASCII 也走同一条路（中文输入法提交的字符）
        assert_eq!(key_from_dom("KeyA", "设"), Some(Key::Char('设')));
    }

    #[test]
    fn maps_space_to_explicit_space_char() {
        // 空格在 Key 里有专门约定：Key::Char(' ')
        assert_eq!(key_from_dom("Space", " "), Some(Key::Char(' ')));
    }

    #[test]
    fn code_wins_over_key_for_special_keys() {
        // 小键盘回车：code 是 NumpadEnter，key 是 "Enter"，两者都指向 Enter
        assert_eq!(key_from_dom("NumpadEnter", "Enter"), Some(Key::Enter));
        // 反例保护：code 已识别为特殊键时，即使 key 是单字符也不应变成 Char
        assert_eq!(key_from_dom("Enter", "a"), Some(Key::Enter));
    }

    #[test]
    fn rejects_non_printable_and_multi_char_keys() {
        assert_eq!(key_from_dom("ControlLeft", "Control"), None);
        assert_eq!(key_from_dom("ShiftLeft", "Shift"), None);
        assert_eq!(key_from_dom("Unidentified", "Unidentified"), None);
        assert_eq!(key_from_dom("Dead", "Dead"), None);
        assert_eq!(key_from_dom("", ""), None);
    }

    #[test]
    fn rejects_control_characters() {
        // DOM 在部分组合下会把 key 给成 "\t" 之类的单个控制字符
        assert_eq!(key_from_dom("Weird", "\t"), None);
        assert_eq!(key_from_dom("Weird", "\u{7f}"), None);
    }

    // ---- char_from_dom ----

    #[test]
    fn char_from_dom_matches_key_mapping_for_printables() {
        assert_eq!(char_from_dom("a"), Some('a'));
        assert_eq!(char_from_dom("8"), Some('8'));
        assert_eq!(char_from_dom("设"), Some('设'));
        assert_eq!(char_from_dom(" "), Some(' '));
    }

    #[test]
    fn char_from_dom_ignores_named_keys() {
        assert_eq!(char_from_dom("Enter"), None);
        assert_eq!(char_from_dom("ArrowUp"), None);
        assert_eq!(char_from_dom("Shift"), None);
    }

    // ---- button_from_dom ----

    #[test]
    fn maps_primary_middle_secondary_buttons() {
        assert_eq!(button_from_dom(0), Some(MouseButton::Left));
        assert_eq!(button_from_dom(1), Some(MouseButton::Middle));
        assert_eq!(button_from_dom(2), Some(MouseButton::Right));
    }

    #[test]
    fn rejects_side_buttons() {
        // 浏览器后退/前进键：框架没有对应语义，宁可丢弃也不当成左右键
        assert_eq!(button_from_dom(3), None);
        assert_eq!(button_from_dom(4), None);
        assert_eq!(button_from_dom(-1), None);
    }

    // ---- mods_from_event ----

    #[test]
    fn maps_modifier_flags_independently() {
        assert_eq!(
            mods_from_event(false, false, false, false),
            Modifiers::default()
        );
        let m = mods_from_event(true, true, true, false);
        assert!(m.ctrl && m.shift && m.alt);
        let only_shift = mods_from_event(false, true, false, false);
        assert!(only_shift.shift && !only_shift.ctrl && !only_shift.alt);
    }

    #[test]
    fn meta_is_deliberately_not_folded_into_ctrl() {
        // ⌘ 不应变成 Ctrl：macOS 上 Cmd+C 与 Ctrl+C 语义不同
        let m = mods_from_event(false, false, false, true);
        assert!(!m.ctrl && !m.shift && !m.alt);
    }

    // ---- wheel_delta_from_dom ----

    #[test]
    fn wheel_delta_is_negated() {
        // DOM 向下为正；框架向上为正
        assert!((wheel_delta_from_dom(100.0) + 100.0).abs() < f32::EPSILON);
        assert!((wheel_delta_from_dom(-100.0) - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn wheel_delta_of_non_finite_values_is_zero() {
        assert_eq!(wheel_delta_from_dom(f64::NAN), 0.0);
        assert_eq!(wheel_delta_from_dom(f64::INFINITY), 0.0);
    }

    // ---- to_ui_y ----

    #[test]
    fn top_of_canvas_maps_to_last_ui_row() {
        // 屏幕最上一行（offsetY = 0）对应 UI 坐标的最大 y。
        assert!((to_ui_y(600.0, 0.0) - 599.0).abs() < f32::EPSILON);
    }

    #[test]
    fn bottom_of_canvas_maps_to_zero() {
        assert!((to_ui_y(600.0, 599.0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mapping_is_symmetric_around_the_middle() {
        let h = 600.0;
        for y in [0.0_f32, 1.0, 100.0, 299.0, 300.0, 599.0] {
            let mirrored = to_ui_y(h, y);
            let back = to_ui_y(h, mirrored);
            assert!((back - y).abs() < f32::EPSILON, "y={y} back={back}");
        }
    }

    #[test]
    fn out_of_range_values_are_clamped_or_safe() {
        // 鼠标拖出画布时 offsetY 可能超出范围：不能产生负数坐标。
        assert_eq!(to_ui_y(600.0, 900.0), 0.0);
        assert_eq!(to_ui_y(f32::NAN, 10.0), 0.0);
        assert_eq!(to_ui_y(600.0, f32::NAN), 0.0);
    }
}
