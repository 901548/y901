//! 键盘：keycode → keysym → `y901_ui::Key` / `char`。
//!
//! ## 为什么要两步
//!
//! X11 的 `KeyPress` 事件只告诉你 **keycode**（一个物理键位的编号，如 38）。
//! 它是**布局相关**的：keycode 38 在 US 布局上是 `a`，在德语布局上可能是 `a` 也可能不是。
//! 所以必须先向服务器要 **keyboard mapping 表**（keycode → keysym），
//! 再把 keysym 译成字符/功能键。
//!
//! ## 本轮覆盖范围（与约定一致）
//!
//! ASCII 32~126 + Enter / Backspace / Tab / Escape / 方向键（+ Delete/Home/End/PageUp/PageDown）。
//! **不做**：死键、组合字符、非 Latin-1 布局、IME。

use y901_ui::Key;

/// 一张 keyboard mapping 表：keycode → 若干个 keysym（按修饰键分列）。
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    /// 本表的第一个 keycode（通常 8）
    pub first_keycode: u8,
    /// 每个 keycode 有几个 keysym（通常 2~7）
    pub keysyms_per_keycode: u8,
    /// 扁平存储：`keysyms[i * keysyms_per_keycode + col]`
    pub keysyms: Vec<u32>,
}

impl Keymap {
    /// 取某个 keycode 在指定列上的 keysym。
    pub fn keysym(&self, keycode: u8, col: usize) -> Option<u32> {
        if self.keysyms_per_keycode == 0 {
            return None;
        }
        let idx = keycode.checked_sub(self.first_keycode)? as usize;
        let per = self.keysyms_per_keycode as usize;
        if col >= per {
            return None;
        }
        self.keysyms.get(idx * per + col).copied()
    }

    /// 取"普通"与"Shift"两列的 keysym（列 0/1）。
    ///
    /// 为什么只取两列：第 2 列之后是 AltGr（欧洲字符）与
    /// 组合键，本轮不做；它们对"输入命令行"几乎没影响。
    pub fn plain_and_shift(&self, keycode: u8) -> (Option<u32>, Option<u32>) {
        (self.keysym(keycode, 0), self.keysym(keycode, 1))
    }

    /// 统计：表里有多少个可输出 ASCII 可打印字符的键位。
    pub fn ascii_coverage(&self) -> usize {
        if self.keysyms_per_keycode == 0 {
            return 0;
        }
        // 只遍历**表里真正存在**的 keycode 数（len / per），
        // 而不是猜 256 个——猜大了既浪费又容易把表尾的 0 当成有效键。
        let n_keycodes = self.keysyms.len() / self.keysyms_per_keycode as usize;
        let mut n = 0;
        for idx in 0..n_keycodes {
            let ks = self.keysyms[idx * self.keysyms_per_keycode as usize];
            if (0x20..=0x7E).contains(&ks) {
                n += 1;
            }
        }
        n
    }
}

/// 把 keysym 译成 `y901_ui::Key`。
///
/// 返回：`Some(Key)` 表示这是一个"有意义的键"；`None` 表示本轮不支持。
///
/// 为什么把 ASCII 可打印字符也是 `Key::Char`：y901 的 `Key` 枚举里
/// `Char(char)` 就是"字符键"的表达方式，与 Win32 路径保持一致。
pub fn keysym_to_key(keysym: u32) -> Option<Key> {
    // Latin-1 可打印区：keysym 值直接等于 ASCII/Latin-1 码点
    if (0x20..=0x7E).contains(&keysym) {
        return Some(Key::Char(char::from_u32(keysym)?));
    }
    Some(match keysym {
        0xFF0D | 0xFF8D => Key::Enter, // Return / KP_Enter
        0xFF08 => Key::Backspace,
        0xFF09 | 0xFE20 => Key::Tab, // Tab / ISO_Left_Tab
        0xFF1B => Key::Escape,
        0xFFFF => Key::Delete,
        0xFF51 => Key::Left,
        0xFF52 => Key::Up,
        0xFF53 => Key::Right,
        0xFF54 => Key::Down,
        0xFF50 => Key::Home,
        0xFF57 => Key::End,
        0xFF55 => Key::PageUp,
        0xFF56 => Key::PageDown,
        0xFFBE..=0xFFC9 => Key::Function((keysym - 0xFFBE + 1) as u8), // F1..F12
        0xFFE1 | 0xFFE2 | 0xFFE3 | 0xFFE4 | 0xFFE9 | 0xFFEA => return None, // \u4fee\u9970\u952e\u672c\u8eab\u4e0d\u4ea7\u751f\u5b57\u7b26
        _ => return None,
    })
}

/// 从 keysym 取字符（仅当它是可打印字符时）。
pub fn keysym_to_char(keysym: u32) -> Option<char> {
    if (0x20..=0x7E).contains(&keysym) {
        char::from_u32(keysym)
    } else {
        None
    }
}

/// X11 修饰键位掩码 → `y901_ui::Modifiers`。
///
/// 位定义（X11 规范 §5.2）：Shift=1<<0, Lock=1<<1, Control=1<<2, Mod1(Alt)=1<<3。
pub fn modifiers_from_state(state: u16) -> y901_ui::Modifiers {
    y901_ui::Modifiers {
        shift: state & 0x01 != 0,
        ctrl: state & 0x04 != 0,
        alt: state & 0x08 != 0,
    }
}
