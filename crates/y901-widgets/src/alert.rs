//! Alert：四种类型的瞬时提示条（**最小版**）。
//!
//! 刻意不做：队列、自动消失、堆叠、多条合并 —— 那是一个"通知系统"，
//! 不是单个组件（需要定时器、去重、最大同屏数、悬停暂停）。本组件只负责**画一条**，
//! 何时出现、何时消失由调用方决定。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextMetrics, TextStyle, UiEvent, Widget,
};

/// 提示类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    /// 普通信息（蓝）。
    Info,
    /// 成功（绿）。
    Success,
    /// 警告（黄）。
    Warning,
    /// 错误（红）。
    Error,
}

impl AlertKind {
    /// 该类型的强调色（从主题的语义色取，自动适配 DARK/LIGHT）。
    pub fn accent(self, theme: &y901_ui::Theme) -> Color {
        match self {
            AlertKind::Info => theme.info_color(),
            AlertKind::Success => theme.success_color(),
            AlertKind::Warning => theme.warning_color(),
            AlertKind::Error => theme.error_color(),
        }
    }

    /// 语义标签。
    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::Info => "info",
            AlertKind::Success => "success",
            AlertKind::Warning => "warning",
            AlertKind::Error => "error",
        }
    }
}

/// 左侧色条宽度。
const BAR_W: f32 = 3.0;
/// 单行 / 带说明两种高度。
const H_ONE_LINE: f32 = 48.0;
const H_WITH_DESC: f32 = 64.0;
/// 右侧关闭按钮的命中尺寸。
const DISMISS_W: f32 = 32.0;

/// 一条提示。
pub struct Alert {
    id: String,
    kind: AlertKind,
    title: String,
    description: Option<String>,
    /// 点×时发出的命令。
    command: Option<y901_ui::Command>,
    rect: Rect,
    item: FlexItem,
    /// 可选样式覆盖（仅容器型表面）。
    style: Option<crate::style::WidgetStyle>,
    /// **DPI 缩放**（`measure` 里唯一写入点；未 measure 时 1.0 = 兜底）。
    scale: f32,
    dismiss_rect: Rect,
}

impl Alert {
    /// 创建一条提示。
    pub fn new(id: impl Into<String>, kind: AlertKind, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            description: None,
            command: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            style: None,
            scale: 1.0,
            dismiss_rect: Rect::ZERO,
        }
    }

    /// 追加说明文字（会让高度从 48 变成 64）。
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// 追加"关闭"按钮与回调。
    pub fn on_dismiss(mut self, cmd: y901_ui::Command) -> Self {
        self.command = Some(cmd);
        self
    }

    /// **覆盖容器型表面**（背景 / 描边 / 圆角）。
    ///
    /// 注意：语义色（如 switch 轨道、error 红、empty 灰）**不**由它覆盖 ——
    /// 那些走 `Theme`，否则会破坏 DARK/LIGHT 自适应。
    pub fn style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 类型（测试/示例用）。
    pub fn kind(&self) -> AlertKind {
        self.kind
    }
}

impl Widget for Alert {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        let h = if self.description.is_some() {
            H_WITH_DESC * self.scale
        } else {
            H_ONE_LINE * self.scale
        };
        c.constrain(Size::new(c.max.w, h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        self.dismiss_rect = Rect::new(
            rect.right() - DISMISS_W * self.scale,
            rect.y + (rect.h - DISMISS_W * self.scale) * 0.5,
            DISMISS_W * self.scale,
            DISMISS_W * self.scale,
        );
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if let UiEvent::MouseUp { pos, .. } = event {
            if self.dismiss_rect.contains(*pos) {
                if let Some(cmd) = self.command.as_ref() {
                    let c = crate::command_with_args(cmd, y901_ui::json::Json::Null);
                    ctx.emit_command(c);
                }
                return EventResult::Redraw;
            }
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let accent = {
            let theme = ctx.theme();
            self.kind.accent(theme)
        };
        // 背景：主题底色 + 一层非常淡的 kind 色偏
        let (bg, text_fg, muted) = {
            let theme = ctx.theme();
            (
                theme.style(Role::PANEL, State::Hovered).bg,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.muted_text_color(),
            )
        };
        // **样式覆盖**：只接管底色/描边/圆角，强调色条仍走语义色。
        let bg = self.style.and_then(|s| s.bg).unwrap_or(bg);
        let tint = Color::rgba(accent.r, accent.g, accent.b, 0.10);
        ctx.fill_rect(self.rect, bg);
        ctx.fill_rect(self.rect, tint);
        // 左侧调整色条
        ctx.fill_rect(
            Rect::new(self.rect.x, self.rect.y, BAR_W * self.scale, self.rect.h),
            accent,
        );

        let text_x = self.rect.x + (BAR_W + 12.0) * self.scale;
        let one_line = self.description.is_none();
        let title_style = TextStyle::fg(text_fg);
        let ty = if one_line {
            self.rect.y + (self.rect.h - ctx.line_height(&title_style)) * 0.5
        } else {
            self.rect.y + 12.0
        };
        ctx.draw_text(Pos::new(text_x, ty), &self.title, &title_style);

        if let Some(desc) = self.description.as_ref() {
            let style = TextStyle::fg(muted);
            ctx.draw_text(Pos::new(text_x, ty + 22.0 * self.scale), desc, &style);
        }

        if self.command.is_some() {
            // 关闭：一个×（用文字画，不新增图元）
            let style = TextStyle::fg(muted);
            let w = ctx.measure_text("×", &style).w;
            ctx.draw_text(
                Pos::new(
                    self.dismiss_rect.x + (self.dismiss_rect.w - w) * 0.5,
                    self.dismiss_rect.y + (self.dismiss_rect.h - ctx.line_height(&style)) * 0.5,
                ),
                "×",
                &style,
            );
        }
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(true)
            .label(format!("alert {}", self.id))
            .value(self.title.clone())
            .prop("kind", self.kind.as_str())
            .action("dismiss");
        if let Some(d) = self.description.as_ref() {
            s = s.prop("description", d.clone());
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use y901_ui::Color;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    #[test]
    fn height_depends_on_description() {
        let mut a = Alert::new("a", AlertKind::Info, "hi");
        let mut b = Alert::new("b", AlertKind::Warning, "hi").description("details");
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(400.0, 100.0));
        assert_eq!(a.measure(&Metrics, c, &mut cache).h, H_ONE_LINE);
        assert_eq!(b.measure(&Metrics, c, &mut cache).h, H_WITH_DESC);
    }

    #[test]
    fn dismiss_fires_only_inside_its_rect() {
        let mut a = Alert::new("a", AlertKind::Error, "boom").on_dismiss(y901_ui::Command::app(
            "alert:dismiss",
            y901_ui::json::Json::Null,
        ));
        let mut cache = LayoutCache::new();
        a.arrange(Rect::new(0.0, 0.0, 400.0, 48.0), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 48.0));
        a.event(
            &mut ctx,
            &UiEvent::MouseUp {
                pos: Pos::new(10.0, 10.0),
                button: y901_ui::MouseButton::Left,
            },
        );
        assert!(ctx.commands.is_empty(), "点外部不应发命令");
        let r = a.dismiss_rect;
        a.event(
            &mut ctx,
            &UiEvent::MouseUp {
                pos: Pos::new(r.x + 4.0, r.y + 4.0),
                button: y901_ui::MouseButton::Left,
            },
        );
        assert_eq!(ctx.commands.len(), 1, "点×应发出命令");
    }

    #[test]
    fn style_is_stored_for_container_surface() {
        let st = crate::style::WidgetStyle::default()
            .bg(Color::rgba(0.1, 0.2, 0.3, 1.0))
            .radius(3.0);
        let a = Alert::new("a", AlertKind::Info, "t").style(st);
        assert_eq!(a.style, Some(st), "style 应被保存（render 里用它覆盖底色）");
    }
}
