//! 颜色与文本样式。框架只描述"要什么颜色"，不关心怎么上屏。

/// 线性空间的 RGBA，0.0..=1.0。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// 红通道（0..=1 线性空间，非 sRGB 编码值）。
    pub r: f32,
    /// 绿通道。
    pub g: f32,
    /// 蓝通道。
    pub b: f32,
    /// 不透明度；0 = 完全透明。渲染走预乘 alpha，混合时按覆盖率叠加。
    pub a: f32,
}

impl Color {
    /// 完全透明（也用于\「不绘制背景\「：`bg.a == 0` 时后端跳过背景填充）。
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    /// 纯黑。
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    /// 纯白；`TextStyle::default()` 的前景就是它（曾经误用透明色导致文字不可见）。
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);

    /// 由 0..=1 的三个通道构造，alpha 为 1。
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// 由 0..=1 的四个通道构造。
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// 由 0..=255 的字节构造，便于直接写设计稿上的颜色值。
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }

    /// 线性插值：`t = 0` 返回自己，`t = 1` 返回 `other`（含 alpha）。
    ///
    /// 用途：渐变背景按条带取中间色。
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color::rgba(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
            self.a + (other.a - self.a) * t,
        )
    }

    /// 返回同一颜色的新副本，alpha 换成 `a`（0..=1）。
    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// 反显：前景当背景、背景当前景。
    pub fn over(self, backdrop: Color) -> Color {
        let a = self.a + backdrop.a * (1.0 - self.a);
        if a <= 0.0 {
            return Color::TRANSPARENT;
        }
        Color {
            r: (self.r * self.a + backdrop.r * backdrop.a * (1.0 - self.a)) / a,
            g: (self.g * self.a + backdrop.g * backdrop.a * (1.0 - self.a)) / a,
            b: (self.b * self.a + backdrop.b * backdrop.a * (1.0 - self.a)) / a,
            a,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::TRANSPARENT
    }
}

/// 文本样式。框架层只表达语义，字形如何光栅化由后端决定。
///
/// **`Default` 的前景是白色**，不是 `Color::default()`（透明）。
/// 实测教训：之前直接 derive `Default`，于是 `TextStyle::default()` 写出的文字**完全不可见**，
/// 而且从像素上看就像"后端没画字"——我曾经因此误判 SoftBackend 不支持字形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// 前景色。
    pub fg: Color,
    /// 背景色；`TRANSPARENT` 表示不画背景（让下层透出来）。
    pub bg: Color,
    /// 粗体。
    pub bold: bool,
    /// 斜体。
    pub italic: bool,
    /// 下划线。
    pub underline: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            fg: Color::WHITE,
            bg: Color::TRANSPARENT,
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

impl TextStyle {
    /// 只设前景色，背景保持透明——大多数场景只需要这一项。
    pub const fn fg(color: Color) -> Self {
        Self {
            fg: color,
            bg: Color::TRANSPARENT,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    /// 设置背景色并返回自身（构建器风格）。
    pub fn with_bg(mut self, color: Color) -> Self {
        self.bg = color;
        self
    }

    /// 打开粗体。
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// 打开下划线。
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }
}
