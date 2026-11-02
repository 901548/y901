//! 主题契约：把颜色/尺寸从组件里抽出来，按 `(角色, 状态)` 查表。
//!
//! 刻意做成**固定数组**而不是 HashMap：`Theme::DARK` 必须是 `const`，
//! 这样 `RenderCtx::theme()` 能返回 `&'static`，组件零成本查表。

use crate::color::Color;
use crate::geometry::Edges;
use crate::widget::Role;

/// 组件的交互状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    /// 常态。
    Normal,
    /// 悬停态（鼠标在上）。
    Hovered,
    /// 按下态。
    Pressed,
    /// 键盘焦点态。
    Focused,
    /// 禁用态；同时应拒绝动作。
    Disabled,
    /// 持久选中态（按钮组单选）。**没有独立主题列**——由 Normal 向 Pressed 混色派生。
    Selected,
}

impl State {
    /// 主题表的列数（`Normal..=Disabled` 五列；`Selected` 是派生状态，不占列）。
    pub const COUNT: usize = 5;

    /// 主题表列下标。`Selected` 没有列，返回 `None`。
    pub const fn table_index(self) -> Option<usize> {
        match self {
            State::Normal => Some(0),
            State::Hovered => Some(1),
            State::Pressed => Some(2),
            State::Focused => Some(3),
            State::Disabled => Some(4),
            State::Selected => None,
        }
    }
}

/// 一套视觉属性。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// 前景色（文字/图形）。
    pub fg: Color,
    /// 背景色。
    pub bg: Color,
    /// 边框色。
    pub border: Color,
    /// 圆角半径（像素）。
    pub radius: f32,
    /// 内边距。
    pub padding: Edges,
    /// 选区高亮色。
    pub selection: Color,
    /// 光标颜色。
    pub caret: Color,
}

impl Style {
    /// 只给前景与背景，其余取中性默认值——最常用的构造入口。
    pub const fn new(fg: Color, bg: Color) -> Self {
        Self {
            fg,
            bg,
            border: Color::TRANSPARENT,
            radius: 0.0,
            padding: Edges::ZERO,
            selection: Color::rgba(0.24, 0.45, 0.75, 0.55),
            caret: Color::rgba(0.85, 0.90, 1.0, 1.0),
        }
    }

    /// 把背景与边框向 `other` 混色 `t`（0..=1），其余字段保持不变。
    ///
    /// 用途：`State::Selected` 的派生（Normal → Pressed 方向混 55%）。
    /// 输入：`other` 目标样式、`t` 混合比例（越界自动 clamp）。
    /// 返回：新 `Style`（Copy，无分配）。
    pub fn blend_toward(self, other: Style, t: f32) -> Style {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: Color, b: Color| {
            Color::rgba(
                a.r + (b.r - a.r) * t,
                a.g + (b.g - a.g) * t,
                a.b + (b.b - a.b) * t,
                a.a + (b.a - a.a) * t,
            )
        };
        Style {
            bg: mix(self.bg, other.bg),
            border: mix(self.border, other.border),
            ..self
        }
    }

    /// 设置内边距并返回自身。
    pub const fn with_padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    /// 设置圆角半径并返回自身。
    pub const fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

/// 主题：`Role` → 槽位，`State` → 列。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    slots: [&'static str; Theme::SLOTS],
    table: [[Style; State::COUNT]; Theme::SLOTS],
}

impl Theme {
    /// 槽位数。新增角色必须落在这几个槽里，否则会被折叠到 panel。
    pub const SLOTS: usize = 6;

    /// 把任意角色映射到槽位；未知角色落到 "panel"。
    pub const fn slot(role: Role) -> usize {
        // Role 是开放字符串，只能线性比对（6 次 str 比较，可忽略）
        let name = role.0;
        let mut i = 0;
        while i < Theme::SLOTS {
            if str_eq(name, Theme::slot_name(i)) {
                return i;
            }
            i += 1;
        }
        0
    }

    /// 槽位名；主题文件与调试输出用它做稳定标识。
    pub const fn slot_name(i: usize) -> &'static str {
        match i {
            0 => "panel",
            1 => "button",
            2 => "text_input",
            3 => "text",
            4 => "list",
            _ => "scrollbar",
        }
    }

    /// 查表。未知角色回退到 panel。
    ///
    /// `State::Selected`（持久选中）没有独立列：由该角色的 `Normal` 向 `Pressed`
    /// 混色 55% 得到——比 Normal 深、比 Pressed 浅，符合"选中"的视觉预期。
    /// 这样所有角色都自动拥有选中态，不必给 12 行主题表各加一列。
    pub fn style(&self, role: Role, state: State) -> Style {
        let slot = Theme::slot(role);
        match state.table_index() {
            Some(index) => self.table[slot][index],
            None => {
                let normal = self.table[slot][State::Normal.table_index().unwrap_or(0)];
                let pressed = self.table[slot][State::Pressed.table_index().unwrap_or(2)];
                normal.blend_toward(pressed, 0.55)
            }
        }
    }

    /// 默认暗色主题。组件不得再硬编码颜色。
    pub const DARK: Theme = Theme {
        slots: ["panel", "button", "text_input", "text", "list", "scrollbar"],
        table: [
            // panel
            [
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.05, 0.05, 0.07, 1.0),
                ),
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.08, 0.08, 0.10, 1.0),
                ),
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.10, 0.10, 0.13, 1.0),
                ),
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.07, 0.07, 0.09, 1.0),
                ),
                Style::new(
                    Color::rgba(0.50, 0.52, 0.58, 1.0),
                    Color::rgba(0.05, 0.05, 0.07, 1.0),
                ),
            ],
            // button
            [
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.16, 0.18, 0.24, 1.0),
                )
                .with_padding(Edges::symmetric(12.0, 6.0))
                .with_radius(4.0),
                Style::new(
                    Color::rgba(0.95, 0.97, 1.0, 1.0),
                    Color::rgba(0.22, 0.26, 0.34, 1.0),
                )
                .with_padding(Edges::symmetric(12.0, 6.0))
                .with_radius(4.0),
                Style::new(
                    Color::rgba(1.0, 1.0, 1.0, 1.0),
                    Color::rgba(0.30, 0.42, 0.60, 1.0),
                )
                .with_padding(Edges::symmetric(12.0, 6.0))
                .with_radius(4.0),
                Style::new(
                    Color::rgba(1.0, 1.0, 1.0, 1.0),
                    Color::rgba(0.20, 0.30, 0.45, 1.0),
                )
                .with_padding(Edges::symmetric(12.0, 6.0))
                .with_radius(4.0),
                Style::new(
                    Color::rgba(0.55, 0.56, 0.60, 1.0),
                    Color::rgba(0.12, 0.13, 0.16, 1.0),
                )
                .with_padding(Edges::symmetric(12.0, 6.0))
                .with_radius(4.0),
            ],
            // text_input
            [
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.10, 0.11, 0.14, 1.0),
                )
                .with_padding(Edges::symmetric(8.0, 4.0)),
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.12, 0.13, 0.17, 1.0),
                )
                .with_padding(Edges::symmetric(8.0, 4.0)),
                Style::new(
                    Color::rgba(0.90, 0.92, 0.96, 1.0),
                    Color::rgba(0.13, 0.14, 0.18, 1.0),
                )
                .with_padding(Edges::symmetric(8.0, 4.0)),
                Style::new(
                    Color::rgba(0.96, 0.98, 1.0, 1.0),
                    Color::rgba(0.10, 0.11, 0.14, 1.0),
                )
                .with_padding(Edges::symmetric(8.0, 4.0)),
                Style::new(
                    Color::rgba(0.55, 0.56, 0.60, 1.0),
                    Color::rgba(0.08, 0.08, 0.10, 1.0),
                )
                .with_padding(Edges::symmetric(8.0, 4.0)),
            ],
            // text
            [
                Style::new(Color::rgba(0.88, 0.90, 0.94, 1.0), Color::TRANSPARENT),
                Style::new(Color::rgba(0.88, 0.90, 0.94, 1.0), Color::TRANSPARENT),
                Style::new(Color::rgba(0.88, 0.90, 0.94, 1.0), Color::TRANSPARENT),
                Style::new(Color::rgba(0.96, 0.98, 1.0, 1.0), Color::TRANSPARENT),
                Style::new(Color::rgba(0.50, 0.52, 0.58, 1.0), Color::TRANSPARENT),
            ],
            // list
            [
                Style::new(
                    Color::rgba(0.86, 0.88, 0.92, 1.0),
                    Color::rgba(0.06, 0.06, 0.08, 1.0),
                ),
                Style::new(
                    Color::rgba(0.90, 0.93, 0.98, 1.0),
                    Color::rgba(0.12, 0.14, 0.18, 1.0),
                ),
                Style::new(
                    Color::rgba(1.0, 1.0, 1.0, 1.0),
                    Color::rgba(0.16, 0.20, 0.28, 1.0),
                ),
                Style::new(
                    Color::rgba(1.0, 1.0, 1.0, 1.0),
                    Color::rgba(0.10, 0.12, 0.16, 1.0),
                ),
                Style::new(
                    Color::rgba(0.50, 0.52, 0.58, 1.0),
                    Color::rgba(0.06, 0.06, 0.08, 1.0),
                ),
            ],
            // scrollbar
            [
                Style::new(Color::TRANSPARENT, Color::rgba(0.22, 0.24, 0.30, 1.0)),
                Style::new(Color::TRANSPARENT, Color::rgba(0.28, 0.31, 0.38, 1.0)),
                Style::new(Color::TRANSPARENT, Color::rgba(0.38, 0.43, 0.55, 1.0)),
                Style::new(Color::TRANSPARENT, Color::rgba(0.22, 0.24, 0.30, 1.0)),
                Style::new(Color::TRANSPARENT, Color::rgba(0.15, 0.16, 0.20, 1.0)),
            ],
        ],
    };
}

/// 高对比主题：证明"换肤"是真的（组件不持有任何颜色）。
pub const LIGHT: Theme = Theme {
    slots: ["panel", "button", "text_input", "text", "list", "scrollbar"],
    table: [
        [
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.96, 0.96, 0.97, 1.0),
            ),
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.94, 0.94, 0.96, 1.0),
            ),
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.92, 0.92, 0.94, 1.0),
            ),
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.95, 0.95, 0.96, 1.0),
            ),
            Style::new(
                Color::rgba(0.45, 0.45, 0.48, 1.0),
                Color::rgba(0.96, 0.96, 0.97, 1.0),
            ),
        ],
        [
            Style::new(
                Color::rgba(0.12, 0.12, 0.14, 1.0),
                Color::rgba(0.88, 0.89, 0.92, 1.0),
            )
            .with_padding(Edges::symmetric(12.0, 6.0))
            .with_radius(4.0),
            Style::new(
                Color::rgba(0.12, 0.12, 0.14, 1.0),
                Color::rgba(0.82, 0.85, 0.92, 1.0),
            )
            .with_padding(Edges::symmetric(12.0, 6.0))
            .with_radius(4.0),
            Style::new(
                Color::rgba(1.0, 1.0, 1.0, 1.0),
                Color::rgba(0.30, 0.45, 0.75, 1.0),
            )
            .with_padding(Edges::symmetric(12.0, 6.0))
            .with_radius(4.0),
            Style::new(
                Color::rgba(1.0, 1.0, 1.0, 1.0),
                Color::rgba(0.35, 0.50, 0.80, 1.0),
            )
            .with_padding(Edges::symmetric(12.0, 6.0))
            .with_radius(4.0),
            Style::new(
                Color::rgba(0.55, 0.55, 0.58, 1.0),
                Color::rgba(0.90, 0.90, 0.92, 1.0),
            )
            .with_padding(Edges::symmetric(12.0, 6.0))
            .with_radius(4.0),
        ],
        [
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.99, 0.99, 1.0, 1.0),
            )
            .with_padding(Edges::symmetric(8.0, 4.0)),
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.97, 0.97, 0.99, 1.0),
            )
            .with_padding(Edges::symmetric(8.0, 4.0)),
            Style::new(
                Color::rgba(0.10, 0.10, 0.12, 1.0),
                Color::rgba(0.95, 0.96, 0.99, 1.0),
            )
            .with_padding(Edges::symmetric(8.0, 4.0)),
            Style::new(
                Color::rgba(0.05, 0.05, 0.08, 1.0),
                Color::rgba(0.99, 0.99, 1.0, 1.0),
            )
            .with_padding(Edges::symmetric(8.0, 4.0)),
            Style::new(
                Color::rgba(0.60, 0.60, 0.62, 1.0),
                Color::rgba(0.94, 0.94, 0.95, 1.0),
            )
            .with_padding(Edges::symmetric(8.0, 4.0)),
        ],
        [
            Style::new(Color::rgba(0.14, 0.14, 0.16, 1.0), Color::TRANSPARENT),
            Style::new(Color::rgba(0.14, 0.14, 0.16, 1.0), Color::TRANSPARENT),
            Style::new(Color::rgba(0.14, 0.14, 0.16, 1.0), Color::TRANSPARENT),
            Style::new(Color::rgba(0.05, 0.05, 0.08, 1.0), Color::TRANSPARENT),
            Style::new(Color::rgba(0.60, 0.60, 0.62, 1.0), Color::TRANSPARENT),
        ],
        [
            Style::new(
                Color::rgba(0.16, 0.16, 0.18, 1.0),
                Color::rgba(0.95, 0.95, 0.96, 1.0),
            ),
            Style::new(
                Color::rgba(0.16, 0.16, 0.18, 1.0),
                Color::rgba(0.88, 0.90, 0.95, 1.0),
            ),
            Style::new(
                Color::rgba(1.0, 1.0, 1.0, 1.0),
                Color::rgba(0.72, 0.80, 0.95, 1.0),
            ),
            Style::new(
                Color::rgba(0.05, 0.05, 0.08, 1.0),
                Color::rgba(0.97, 0.97, 0.99, 1.0),
            ),
            Style::new(
                Color::rgba(0.60, 0.60, 0.62, 1.0),
                Color::rgba(0.95, 0.95, 0.96, 1.0),
            ),
        ],
        [
            Style::new(Color::TRANSPARENT, Color::rgba(0.80, 0.81, 0.85, 1.0)),
            Style::new(Color::TRANSPARENT, Color::rgba(0.72, 0.74, 0.80, 1.0)),
            Style::new(Color::TRANSPARENT, Color::rgba(0.60, 0.64, 0.75, 1.0)),
            Style::new(Color::TRANSPARENT, Color::rgba(0.80, 0.81, 0.85, 1.0)),
            Style::new(Color::TRANSPARENT, Color::rgba(0.88, 0.88, 0.90, 1.0)),
        ],
    ],
};

/// `const` 环境下的字符串比较。
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// 供 RenderCtx::theme() 默认返回的静态主题。
pub static DARK_THEME: Theme = Theme::DARK;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_differ() {
        let dark = Theme::DARK.style(Role::BUTTON, State::Normal);
        let light = LIGHT.style(Role::BUTTON, State::Normal);
        assert_ne!(dark.bg, light.bg, "换主题必须真的换颜色");
    }

    #[test]
    fn unknown_role_falls_back_to_panel() {
        let custom = Role("terminal");
        let a = Theme::DARK.style(custom, State::Normal);
        let b = Theme::DARK.style(Role::PANEL, State::Normal);
        assert_eq!(a, b);
    }

    #[test]
    fn states_have_distinct_backgrounds() {
        let normal = Theme::DARK.style(Role::BUTTON, State::Normal).bg;
        let hovered = Theme::DARK.style(Role::BUTTON, State::Hovered).bg;
        let pressed = Theme::DARK.style(Role::BUTTON, State::Pressed).bg;
        assert_ne!(normal, hovered);
        assert_ne!(hovered, pressed);
    }
}
