//! EmptyState：空列表 / 无结果的占位态。
//!
//! 展示：居中的圆形图标占位 + 标题 + 可选说明 + 可选操作按钮。
//! 为什么需要它：真实应用的第一个界面里，"列表为空"和"列表有内容"同样常见；
//! 没有它，外部用户只能自己拼一段文字（或干脆留白）。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Align, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

/// 图标占位直径。
const ICON_D: f32 = 48.0;
/// 操作按钮高度。
const ACTION_H: f32 = 32.0;

/// 空状态占位。
pub struct EmptyState {
    id: String,
    title: String,
    description: Option<String>,
    action: Option<(String, y901_ui::Command)>,
    rect: Rect,
    item: FlexItem,
    /// DPI 缩放（`measure` 里唯一写入；未测过时 1.0）。
    scale: f32,
    action_rect: Rect,
}

impl EmptyState {
    /// 用标题创建（标题必填）。
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            action: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            scale: 1.0,
            action_rect: Rect::ZERO,
        }
    }

    /// 追加说明文字。
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// 追加操作按钮。
    pub fn action(mut self, label: impl Into<String>, cmd: y901_ui::Command) -> Self {
        self.action = Some((label.into(), cmd));
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 期望高度：图标 + 标题 +（说明）+（按钮）+ 间距。
    fn content_height(&self) -> f32 {
        let mut h = (ICON_D + 16.0 + 24.0) * self.scale;
        if self.description.is_some() {
            h += (8.0 + 18.0) * self.scale;
        }
        if self.action.is_some() {
            h += (16.0 + ACTION_H) * self.scale;
        }
        h
    }
}

impl Widget for EmptyState {
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
        c.constrain(Size::new(c.max.w, self.content_height()))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        let w = 120.0 * self.scale;
        self.action_rect = Rect::new(
            rect.x + (rect.w - w) * 0.5,
            rect.bottom() - (ACTION_H + 8.0) * self.scale,
            w,
            ACTION_H * self.scale,
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
            if self.action.is_some() && self.action_rect.contains(*pos) {
                if let Some((_, cmd)) = self.action.as_ref() {
                    ctx.emit_command(crate::command_with_args(cmd, y901_ui::json::Json::Null));
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
        // 先把颜色拷出来：`ctx.theme()` 持有 `&ctx` 的引用，
        // 后续 `ctx.fill_*` 需要 `&mut ctx`（否则 E0502）。
        let (muted, text_fg, icon_bg, accent) = {
            let theme = ctx.theme();
            (
                theme.muted_text_color(),
                theme.style(Role::TEXT, State::Normal).fg,
                theme.style(Role::PANEL, State::Hovered).bg,
                theme.style(Role::BUTTON, State::Pressed).bg,
            )
        };

        let mut y = self.rect.y + 8.0;
        // 图标占位：一个圆
        let cx = self.rect.x + self.rect.w * 0.5;
        ctx.fill_round_rect(
            Rect::new(cx - ICON_D * self.scale * 0.5, y, ICON_D * self.scale, ICON_D * self.scale),
            icon_bg,
            (ICON_D * self.scale) * 0.5,
        );
        y += (ICON_D + 16.0) * self.scale;

        // 标题（居中）
        let title_style = TextStyle::fg(text_fg);
        let tw = ctx.measure_text(&self.title, &title_style).w;
        ctx.draw_text(
            Pos::new(cx - tw * 0.5, y),
            &self.title,
            &title_style,
        );
        y += 24.0 * self.scale;

        // 说明（居中）
        if let Some(desc) = self.description.as_ref() {
            y += 8.0 * self.scale;
            let style = TextStyle::fg(muted);
            let dw = ctx.measure_text(desc, &style).w;
            ctx.draw_text(Pos::new(cx - dw * 0.5, y), desc, &style);
        }

        // 操作按钮（一个居中的圆角矩形 + 文字）
        if let Some((label, _)) = self.action.as_ref() {
            ctx.fill_round_rect(self.action_rect, accent, 6.0);
            let style = TextStyle::fg(y901_ui::Color::WHITE);
            let lw = ctx.measure_text(label, &style).w;
            let ly = self.action_rect.y + (ACTION_H * self.scale - ctx.line_height(&style)) * 0.5;
            ctx.draw_text(
                Pos::new(self.action_rect.x + (self.action_rect.w - lw) * 0.5, ly),
                label,
                &style,
            );
        }
        let _ = Align::Center;
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(true)
            .label(format!("empty {}", self.id))
            .value(self.title.clone())
            .action("action");
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
    fn height_grows_with_optional_parts() {
        let mut a = EmptyState::new("e", "暂无数据");
        let mut b = EmptyState::new("e", "暂无数据")
            .description("点击上方按钮创建")
            .action("确定", y901_ui::Command::app("empty_state:activate", y901_ui::json::Json::Null));
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(400.0, 400.0));
        let ha = a.measure(&Metrics, c, &mut cache).h;
        let hb = b.measure(&Metrics, c, &mut cache).h;
        assert!(hb > ha, "说明与按钮应增加高度：{ha} vs {hb}");
    }

    #[test]
    fn action_only_fires_inside_its_rect() {
        
        let mut e = EmptyState::new("e", "t").action("确定", y901_ui::Command::app("empty_state:activate", y901_ui::json::Json::Null));
        let mut cache = LayoutCache::new();
        e.arrange(Rect::new(0.0, 0.0, 400.0, 200.0), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 200.0));
        // 在按钮外点一下：不触发
        e.event(&mut ctx, &UiEvent::MouseUp { pos: Pos::new(4.0, 4.0), button: y901_ui::MouseButton::Left });
        assert!(ctx.commands.is_empty());
        // 在按钮内点一下：触发
        let c = e.action_rect;
        e.event(&mut ctx, &UiEvent::MouseUp { pos: Pos::new(c.x + 4.0, c.y + 4.0), button: y901_ui::MouseButton::Left });
        assert_eq!(ctx.commands.len(), 1);
    }
}