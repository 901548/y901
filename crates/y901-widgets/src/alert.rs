//! Alert：四种类型的瞬时提示条（**最小版**）。
//!
//! 刻意不做：队列、自动消失、堆叠、多条合并 —— 那是一个"通知系统"，
//! 不是单个组件（需要定时器、去重、最大同屏数、悬停暂停）。本组件只负责**画一条**，
//! 何时出现、何时消失由调用方决定。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
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
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let h = if self.description.is_some() {
            H_WITH_DESC
        } else {
            H_ONE_LINE
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
            rect.right() - DISMISS_W,
            rect.y + (rect.h - DISMISS_W) * 0.5,
            DISMISS_W,
            DISMISS_W,
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
        // \u80cc\u666f\uff1a\u4e3b\u9898\u5e95\u8272 + \u4e00\u5c42\u975e\u5e38\u6de1\u7684 kind \u8272\u504f
        let (bg, text_fg, muted) = {
            let theme = ctx.theme();
            (
                theme.style(Role::PANEL, State::Hovered).bg,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.muted_text_color(),
            )
        };
        let tint = Color::rgba(accent.r, accent.g, accent.b, 0.10);
        ctx.fill_rect(self.rect, bg);
        ctx.fill_rect(self.rect, tint);
        // \u5de6\u4fa7\u8c03\u6574\u8272\u6761
        ctx.fill_rect(
            Rect::new(self.rect.x, self.rect.y, BAR_W, self.rect.h),
            accent,
        );

        let text_x = self.rect.x + BAR_W + 12.0;
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
            ctx.draw_text(Pos::new(text_x, ty + 22.0), desc, &style);
        }

        if self.command.is_some() {
            // \u5173\u95ed\uff1a\u4e00\u4e2a\u00d7\uff08\u7528\u6587\u5b57\u753b\uff0c\u4e0d\u65b0\u589e\u56fe\u5143\uff09
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
            .prop("kind", self.kind.as_str());
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
        
        let mut a = Alert::new("a", AlertKind::Error, "boom").on_dismiss(y901_ui::Command::app("alert:dismiss", y901_ui::json::Json::Null));
        let mut cache = LayoutCache::new();
        a.arrange(Rect::new(0.0, 0.0, 400.0, 48.0), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 48.0));
        a.event(&mut ctx, &UiEvent::MouseUp { pos: Pos::new(10.0, 10.0), button: y901_ui::MouseButton::Left });
        assert!(ctx.commands.is_empty(), "点外部不应发命令");
        let r = a.dismiss_rect;
        a.event(&mut ctx, &UiEvent::MouseUp { pos: Pos::new(r.x + 4.0, r.y + 4.0), button: y901_ui::MouseButton::Left });
        assert_eq!(ctx.commands.len(), 1, "点×应发出命令");
    }
}