//! \u952e\u76d8\uff1akeycode \u2192 keysym \u2192 `y901_ui::Key` / `char`\u3002
//!
//! ## \u4e3a\u4ec0\u4e48\u8981\u4e24\u6b65
//!
//! X11 \u7684 `KeyPress` \u4e8b\u4ef6\u53ea\u544a\u8bc9\u4f60 **keycode**\uff08\u4e00\u4e2a\u7269\u7406\u952e\u4f4d\u7684\u7f16\u53f7\uff0c\u5982 38\uff09\u3002
//! \u5b83\u662f**\u5e03\u5c40\u76f8\u5173**\u7684\uff1akeycode 38 \u5728 US \u5e03\u5c40\u4e0a\u662f `a`\uff0c\u5728\u5fb7\u8bed\u5e03\u5c40\u4e0a\u53ef\u80fd\u662f `a` \u4e5f\u53ef\u80fd\u4e0d\u662f\u3002
//! \u6240\u4ee5\u5fc5\u987b\u5148\u5411\u670d\u52a1\u5668\u8981 **keyboard mapping \u8868**\uff08keycode \u2192 keysym\uff09\uff0c
//! \u518d\u628a keysym \u8bd1\u6210\u5b57\u7b26/\u529f\u80fd\u952e\u3002
//!
//! ## \u672c\u8f6e\u8986\u76d6\u8303\u56f4\uff08\u4e0e\u7ea6\u5b9a\u4e00\u81f4\uff09
//!
//! ASCII 32~126 + Enter / Backspace / Tab / Escape / \u65b9\u5411\u952e\uff08+ Delete/Home/End/PageUp/PageDown\uff09\u3002
//! **\u4e0d\u505a**\uff1a\u6b7b\u952e\u3001\u7ec4\u5408\u5b57\u7b26\u3001\u975e Latin-1 \u5e03\u5c40\u3001IME\u3002

use y901_ui::Key;

/// \u4e00\u5f20 keyboard mapping \u8868\uff1akeycode \u2192 \u82e5\u5e72\u4e2a keysym\uff08\u6309\u4fee\u9970\u952e\u5206\u5217\uff09\u3002
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    /// \u672c\u8868\u7684\u7b2c\u4e00\u4e2a keycode\uff08\u901a\u5e38 8\uff09
    pub first_keycode: u8,
    /// \u6bcf\u4e2a keycode \u6709\u51e0\u4e2a keysym\uff08\u901a\u5e38 2~7\uff09
    pub keysyms_per_keycode: u8,
    /// \u6241\u5e73\u5b58\u50a8\uff1a`keysyms[i * keysyms_per_keycode + col]`
    pub keysyms: Vec<u32>,
}

impl Keymap {
    /// \u53d6\u67d0\u4e2a keycode \u5728\u6307\u5b9a\u5217\u4e0a\u7684 keysym\u3002
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

    /// \u53d6"\u666e\u901a"\u4e0e"Shift"\u4e24\u5217\u7684 keysym\uff08\u5217 0/1\uff09\u3002
    ///
    /// \u4e3a\u4ec0\u4e48\u53ea\u53d6\u4e24\u5217\uff1a\u7b2c 2 \u5217\u4e4b\u540e\u662f AltGr\uff08\u6b27\u6d32\u5b57\u7b26\uff09\u4e0e
    /// \u7ec4\u5408\u952e\uff0c\u672c\u8f6e\u4e0d\u505a\uff1b\u5b83\u4eec\u5bf9"\u8f93\u5165\u547d\u4ee4\u884c"\u51e0\u4e4e\u6ca1\u5f71\u54cd\u3002
    pub fn plain_and_shift(&self, keycode: u8) -> (Option<u32>, Option<u32>) {
        (self.keysym(keycode, 0), self.keysym(keycode, 1))
    }

    /// \u7edf\u8ba1\uff1a\u8868\u91cc\u6709\u591a\u5c11\u4e2a\u53ef\u8f93\u51fa ASCII \u53ef\u6253\u5370\u5b57\u7b26\u7684\u952e\u4f4d\u3002
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

/// \u628a keysym \u8bd1\u6210 `y901_ui::Key`\u3002
///
/// \u8fd4\u56de\uff1a`Some(Key)` \u8868\u793a\u8fd9\u662f\u4e00\u4e2a"\u6709\u610f\u4e49\u7684\u952e"\uff1b`None` \u8868\u793a\u672c\u8f6e\u4e0d\u652f\u6301\u3002
///
/// \u4e3a\u4ec0\u4e48\u628a ASCII \u53ef\u6253\u5370\u5b57\u7b26\u4e5f\u662f `Key::Char`\uff1ay901 \u7684 `Key` \u679a\u4e3e\u91cc
/// `Char(char)` \u5c31\u662f"\u5b57\u7b26\u952e"\u7684\u8868\u8fbe\u65b9\u5f0f\uff0c\u4e0e Win32 \u8def\u5f84\u4fdd\u6301\u4e00\u81f4\u3002
pub fn keysym_to_key(keysym: u32) -> Option<Key> {
    // Latin-1 \u53ef\u6253\u5370\u533a\uff1akeysym \u503c\u76f4\u63a5\u7b49\u4e8e ASCII/Latin-1 \u7801\u70b9
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

/// \u4ece keysym \u53d6\u5b57\u7b26\uff08\u4ec5\u5f53\u5b83\u662f\u53ef\u6253\u5370\u5b57\u7b26\u65f6\uff09\u3002
pub fn keysym_to_char(keysym: u32) -> Option<char> {
    if (0x20..=0x7E).contains(&keysym) {
        char::from_u32(keysym)
    } else {
        None
    }
}

/// X11 \u4fee\u9970\u952e\u4f4d\u63a9\u7801 \u2192 `y901_ui::Modifiers`\u3002
///
/// \u4f4d\u5b9a\u4e49\uff08X11 \u89c4\u8303 \u00a75.2\uff09\uff1aShift=1<<0, Lock=1<<1, Control=1<<2, Mod1(Alt)=1<<3\u3002
pub fn modifiers_from_state(state: u16) -> y901_ui::Modifiers {
    y901_ui::Modifiers {
        shift: state & 0x01 != 0,
        ctrl: state & 0x04 != 0,
        alt: state & 0x08 != 0,
    }
}
