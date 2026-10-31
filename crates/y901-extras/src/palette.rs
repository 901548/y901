//! ANSI 调色板与 SGR 状态机。
//!
//! `Cell::style` 是 `y901_core::StyleTable` 的索引；本模块负责把
//! `ColorSpec` 解析成 RGBA，以及把 SGR 参数序列累积成 `CellStyle`。

use y901_core::{CellStyle, ColorSpec};

/// 默认前景（浅灰白）。
pub const DEFAULT_FG: [f32; 4] = [0.90, 0.92, 0.96, 1.0];
/// 默认背景（近黑）。
pub const DEFAULT_BG: [f32; 4] = [0.02, 0.02, 0.03, 1.0];

/// xterm-256 的前 16 色。
const ANSI: [[u8; 3]; 16] = [
    [0x1c, 0x1c, 0x1c], // 0 black
    [0xcc, 0x55, 0x55], // 1 red
    [0x4e, 0xc9, 0x4e], // 2 green
    [0xd4, 0xb0, 0x4a], // 3 yellow
    [0x4a, 0x8c, 0xd4], // 4 blue
    [0xb0, 0x6a, 0xd4], // 5 magenta
    [0x4a, 0xc0, 0xc0], // 6 cyan
    [0xc8, 0xc8, 0xc8], // 7 white
    [0x66, 0x66, 0x66], // 8 bright black
    [0xff, 0x77, 0x77], // 9 bright red
    [0x7c, 0xf0, 0x7c], // 10 bright green
    [0xf0, 0xd0, 0x6a], // 11 bright yellow
    [0x6a, 0xa8, 0xff], // 12 bright blue
    [0xd0, 0x8a, 0xff], // 13 bright magenta
    [0x6a, 0xe0, 0xe0], // 14 bright cyan
    [0xff, 0xff, 0xff], // 15 bright white
];

/// 把颜色解析成 RGBA。`fallback` 给 `ColorSpec::Default` 用。
pub fn resolve(spec: ColorSpec, fallback: [f32; 4]) -> [f32; 4] {
    match spec {
        ColorSpec::Default => fallback,
        ColorSpec::Rgb(r, g, b) => [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
        ColorSpec::Indexed(i) => xterm256(i),
    }
}

/// xterm-256：0-15 标准色，16-231 6×6×6 色立方，232-255 灰阶。
pub fn xterm256(index: u8) -> [f32; 4] {
    let i = index as usize;
    let rgb = if i < 16 {
        ANSI[i]
    } else if i < 232 {
        let n = i - 16;
        let steps = [0u8, 95, 135, 175, 215, 255];
        [steps[n / 36], steps[(n / 6) % 6], steps[n % 6]]
    } else {
        let v = (8 + (i - 232) * 10) as u8;
        [v, v, v]
    };
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        1.0,
    ]
}

/// 扩展色解析的中间状态。
#[derive(Clone, Copy, Debug)]
struct Extended {
    is_fg: bool,
    /// 0 = 还没读到模式；>0 = 还要读几个数据分量
    remaining: u8,
    mode: u8,
    rgb: [u8; 3],
    rgb_len: u8,
}

/// SGR 累积器：把 `CSI ... m` 变成 `CellStyle`。
#[derive(Clone, Copy, Debug)]
pub struct SgrState {
    pub fg: ColorSpec,
    pub bg: ColorSpec,
    pub bold: bool,
    pub underline: bool,
    pub reverse: bool,
    pending: Option<Extended>,
}

impl Default for SgrState {
    fn default() -> Self {
        Self {
            fg: ColorSpec::Default,
            bg: ColorSpec::Default,
            bold: false,
            underline: false,
            reverse: false,
            pending: None,
        }
    }
}

impl SgrState {
    /// 把当前 SGR 状态转成可驻留的 `CellStyle`（去掉了 `pending` 半成品）。
    ///
    /// 返回：`CellStyle`（Copy）。副作用：无。
    pub fn style(&self) -> CellStyle {
        CellStyle {
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
            underline: self.underline,
            reverse: self.reverse,
        }
    }

    /// 喂入一组 SGR 参数。空表示 `CSI m`（等价于 0）。
    pub fn apply(&mut self, params: &[u32]) {
        if params.is_empty() {
            self.reset();
            return;
        }

        for &p in params {
            if let Some(ext) = self.pending.as_mut() {
                if ext.remaining == 0 {
                    // 这一位是模式：5 = 256 色，2 = truecolor
                    match p {
                        5 => {
                            ext.mode = 5;
                            ext.remaining = 1;
                        }
                        2 => {
                            ext.mode = 2;
                            ext.remaining = 3;
                            ext.rgb = [0; 3];
                            ext.rgb_len = 0;
                        }
                        _ => self.pending = None,
                    }
                    continue;
                }

                if ext.mode == 5 {
                    let spec = ColorSpec::Indexed(p.min(255) as u8);
                    if ext.is_fg {
                        self.fg = spec;
                    } else {
                        self.bg = spec;
                    }
                } else {
                    if (ext.rgb_len as usize) < 3 {
                        ext.rgb[ext.rgb_len as usize] = p.min(255) as u8;
                        ext.rgb_len += 1;
                    }
                    if ext.remaining == 1 {
                        let [r, g, b] = ext.rgb;
                        let spec = ColorSpec::Rgb(r, g, b);
                        if ext.is_fg {
                            self.fg = spec;
                        } else {
                            self.bg = spec;
                        }
                    }
                }

                ext.remaining -= 1;
                if ext.remaining == 0 {
                    self.pending = None;
                }
                continue;
            }

            match p {
                0 => self.reset(),
                1 => self.bold = true,
                4 => self.underline = true,
                7 => self.reverse = true,
                22 => self.bold = false,
                24 => self.underline = false,
                27 => self.reverse = false,
                30..=37 => self.fg = ColorSpec::Indexed((p - 30) as u8),
                38 => {
                    self.pending = Some(Extended {
                        is_fg: true,
                        remaining: 0,
                        mode: 0,
                        rgb: [0; 3],
                        rgb_len: 0,
                    })
                }
                39 => self.fg = ColorSpec::Default,
                40..=47 => self.bg = ColorSpec::Indexed((p - 40) as u8),
                48 => {
                    self.pending = Some(Extended {
                        is_fg: false,
                        remaining: 0,
                        mode: 0,
                        rgb: [0; 3],
                        rgb_len: 0,
                    })
                }
                49 => self.bg = ColorSpec::Default,
                90..=97 => self.fg = ColorSpec::Indexed((p - 90 + 8) as u8),
                100..=107 => self.bg = ColorSpec::Indexed((p - 100 + 8) as u8),
                _ => {}
            }
        }
    }

    /// 复位到默认样式（SGR 0）：颜色、粗体、下划线、反显全部清空。
    ///
    /// 返回：无。副作用：原地改写自身；`pending`（未完成的扩展序列）也被丢弃。
    pub fn reset(&mut self) {
        *self = Self {
            pending: self.pending.take(),
            ..Self::default()
        };
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_colors() {
        let mut s = SgrState::default();
        s.apply(&[31]);
        assert_eq!(s.fg, ColorSpec::Indexed(1)); // red
        s.apply(&[42]);
        assert_eq!(s.bg, ColorSpec::Indexed(2)); // green bg
        s.apply(&[0]);
        assert!(s.style().is_plain());
    }

    #[test]
    fn bright_colors() {
        let mut s = SgrState::default();
        s.apply(&[91]);
        assert_eq!(s.fg, ColorSpec::Indexed(9)); // bright red
        s.apply(&[39]);
        assert_eq!(s.fg, ColorSpec::Default);
    }

    #[test]
    fn truecolor_is_fully_consumed() {
        let mut s = SgrState::default();
        // 38;2;R;G;B：5 个参数都必须吃掉，尤其结尾的 0 不能触发 reset。
        s.apply(&[38, 2, 255, 100, 0]);
        assert!(s.pending.is_none());
        assert_eq!(s.fg, ColorSpec::Rgb(255, 100, 0));

        s.apply(&[31]);
        assert_eq!(s.fg, ColorSpec::Indexed(1));
    }

    #[test]
    fn color256_is_indexed() {
        let mut s = SgrState::default();
        s.apply(&[38, 5, 196]);
        assert_eq!(s.fg, ColorSpec::Indexed(196));
    }

    #[test]
    fn flags_set_and_clear() {
        let mut s = SgrState::default();
        s.apply(&[1, 4, 7]);
        let st = s.style();
        assert!(st.bold && st.underline && st.reverse);
        s.apply(&[22, 24, 27]);
        assert!(s.style().is_plain());
    }

    #[test]
    fn xterm256_cube_and_gray() {
        // 196 = 纯红（cube 的 R=5,G=0,B=0）
        let c = xterm256(196);
        assert!(c[0] > 0.99 && c[1] < 0.01 && c[2] < 0.01);
        // 232 起是灰阶
        let g = xterm256(240);
        assert!((g[0] - g[1]).abs() < 1e-6 && (g[1] - g[2]).abs() < 1e-6);
    }
}
