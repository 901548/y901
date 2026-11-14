//! Switch：44x24 的开关，用于设置面板。
//!
//! 视觉：圆角轨道 + 圆形滑块。未选中 = panel Hovered 轨道、滑块靠左；
//! 选中 = accent 轨道（`BUTTON Pressed` 底）、滑块靠右。
//!
//! **动画**：本版滑块**即时**移动。原因：`RenderCtx::animation(key)` 只能**读**本帧快照，
//! 动画走框架：`EventCtx::start_animation` 发起 → `UiTree::animate` 推进 →
//! 每帧快照 → `render` 里 `RenderCtx::animation(key)` 读进度。
//! 等框架补一个"组件内请求动画"的 API 后，这里把 `knob_x` 改成按 progress 插值即可（已在代码里留位）。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, State,
    TextMetrics, UiEvent, Widget,
};

/// 轨道尺寸（逻辑像素）。
const TRACK_W: f32 = 44.0;
const TRACK_H: f32 = 24.0;
/// 滑块与轨道内边距。
const PAD: f32 = 2.0;
/// 滑块过渡时长（毫秒）。
const SLIDE_MS: u64 = 120;

/// 开关控件。
pub struct Switch {
    id: String,
    checked: bool,
    /// 状态变化时发出的命令（args = bool）。见 crate::command_with_args。
    command: Option<y901_ui::Command>,
    rect: Rect,
    item: FlexItem,
}

impl Switch {
    /// 创建开关。
    pub fn new(id: impl Into<String>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            command: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    /// 状态变化时发出的**命令**（参数 = 新的 bool 状态）。
    ///
    /// 与 `Button` 一致：组件只表达意图，不持有应用逻辑。
    pub fn command(mut self, cmd: y901_ui::Command) -> Self {
        self.command = Some(cmd);
        self
    }

    /// 当前状态。
    pub fn checked(&self) -> bool {
        self.checked
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 切换状态并触发回调。
    fn toggle(&mut self) {
        self.checked = !self.checked;
    }

}

impl Switch {
    /// 发出状态变化命令（参数就是当前状态）。
    fn emit(&self, ctx: &mut EventCtx) {
        if let Some(cmd) = self.command.as_ref() {
            let c = crate::command_with_args(cmd, y901_ui::json::Json::from(self.checked));
            ctx.emit_command(c);
        }
    }
}

impl Widget for Switch {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        // \u6ca1\u6709 `Role::CHECKBOX` \u5e38\u91cf\uff08Theme \u53ea\u5b9a\u4e49\u4e86 8 \u4e2a\u69fd\u4f4d\uff09\uff0c
        // \u7528 BUTTON \u69fd\u4f4d\u53d6\u8272\uff0c\u8bed\u4e49\u4e0a\u901a\u8fc7 `prop("checked")` \u8868\u8fbe\u3002
        Role::BUTTON
    }

    fn focusable(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let s = m.scale();
        c.constrain(Size::new(TRACK_W * s, TRACK_H * s))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseUp { pos, .. } if self.rect.contains(*pos) => {
                ctx.request_focus(self.id.clone());
                self.toggle();
                // 滑块过渡：开 → from 0 到 1；关 → from 1 到 0。
                // 走框架动画系统（EventCtx::start_animation → UiTree::animate →
                // 每帧快照 → render 里 ctx.animation(key) 读进度）。
                let to = if self.checked { 1.0 } else { 0.0 };
                let anim = y901_ui::animation::Animate::new(
                    1.0 - to,
                    to,
                    std::time::Duration::from_millis(SLIDE_MS),
                    y901_ui::animation::Easing::EaseOutCubic,
                );
                ctx.start_animation(y901_ui::animation::key_of(&format!("{}-slide", self.id)), anim);
                self.emit(ctx);
                EventResult::Redraw
            }
            UiEvent::Key { key, .. }
                if matches!(key, y901_ui::Key::Enter)
                    || matches!(key, y901_ui::Key::Char(' ')) =>
            {
                self.toggle();
                self.emit(ctx);
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        // \u5148\u53d6\u8272\u518d\u7ed8\u5236\uff08\u907f\u5f00 `&ctx` / `&mut ctx` \u51b2\u7a81\uff09
        let (on_bg, off_bg, knob) = {
            let theme = ctx.theme();
            let panel = theme.style(Role::PANEL, State::Normal).bg;
            // 滑块 = panel 底色的**反色**：DARK → 近白，LIGHT → 近黑，两种主题都有对比。
            let knob = Color::rgba(1.0 - panel.r, 1.0 - panel.g, 1.0 - panel.b, 1.0);
            (
                theme.style(Role::BUTTON, State::Pressed).bg,
                theme.style(Role::PANEL, State::Hovered).bg,
                knob,
            )
        };
        let track = Rect::new(self.rect.x, self.rect.y, self.rect.w, self.rect.h);
        let radius = self.rect.h * 0.5;
        ctx.fill_round_rect(track, if self.checked { on_bg } else { off_bg }, radius);

        // \u6ed1\u5757\uff1a\u76f4\u5f84 = \u9ad8 - 2*pad\uff08\u6309 rect \u5b9e\u9645\u5c3a\u5bf8\u7b97\uff0c\u81ea\u52a8\u9002\u5e94 DPI\uff09
        let d = (self.rect.h - 2.0 * PAD).max(2.0);
        let kx = if self.checked {
            self.rect.x + self.rect.w - PAD - d * 0.5
        } else {
            self.rect.x + PAD + d * 0.5
        };
        ctx.fill_round_rect(
            Rect::new(kx - d * 0.5, self.rect.y + self.rect.h * 0.5 - d * 0.5, d, d),
            knob,
            d * 0.5,
        );
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("switch {}", self.id))
            .value(if self.checked { "on" } else { "off" })
            .prop("checked", self.checked.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &y901_ui::TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    #[test]
    fn measure_is_track_size() {
        let mut sw = Switch::new("s", false);
        let mut cache = LayoutCache::new();
        let size = sw.measure(&Metrics, Constraints::loose(Size::new(200.0, 100.0)), &mut cache);
        assert_eq!((size.w, size.h), (TRACK_W, TRACK_H));
    }

    #[test]
    fn click_inside_toggles_and_emits_command() {
        let mut sw = Switch::new("s", false)
            .command(y901_ui::Command::app("switch:toggle", y901_ui::json::Json::Null));
        let mut cache = LayoutCache::new();
        sw.arrange(Rect::new(0.0, 0.0, 44.0, 24.0), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        // \u5916\u90e8\u70b9\u51fb\uff1a\u4e0d\u53d8
        sw.event(&mut ctx, &UiEvent::MouseUp { pos: y901_ui::Pos::new(200.0, 200.0), button: y901_ui::MouseButton::Left });
        assert!(!sw.checked());
        // \u5185\u90e8\u70b9\u51fb\uff1a\u5207\u6362 + \u56de\u8c03
        sw.event(&mut ctx, &UiEvent::MouseUp { pos: y901_ui::Pos::new(20.0, 12.0), button: y901_ui::MouseButton::Left });
        assert!(sw.checked());
        assert_eq!(ctx.commands.len(), 1, "应发出一条命令");
    }
}