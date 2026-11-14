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
    /// 本次过渡的起始值（0.0 = 左，1.0 = 右）。
    anim_from: f32,
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
            anim_from: 0.0,
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
        // 没有 `Role::CHECKBOX` 常量（Theme 只定义了 8 个槽位），
        // 用 BUTTON 槽位取色，语义上通过 `prop("checked")` 表达。
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
                self.anim_from = 1.0 - to; // 从对立端开始
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
        // 先取色再绘制（避开 `&ctx` / `&mut ctx` 冲突）
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
        // **插值**：进度来自框架动画（事件里 start_animation 发起），不是硬切。
        let key = y901_ui::animation::key_of(&format!("{}-slide", self.id));
        let p = ctx.animation(key).clamp(0.0, 1.0);
        let target = if self.checked { 1.0 } else { 0.0 };
        let t = (self.anim_from + (target - self.anim_from) * p).clamp(0.0, 1.0);
        let blend = |a: Color, b: Color| {
            Color::rgba(
                a.r + (b.r - a.r) * t,
                a.g + (b.g - a.g) * t,
                a.b + (b.b - a.b) * t,
                1.0,
            )
        };
        ctx.fill_round_rect(track, blend(off_bg, on_bg), radius);

        // 滑块：直径 = 高 - 2*pad（按 rect 实际尺寸算，自动适应 DPI）
        let d = (self.rect.h - 2.0 * PAD).max(2.0);
        let travel = (self.rect.w - 2.0 * PAD - d).max(0.0);
        let kx = self.rect.x + PAD + d * 0.5 + travel * t;
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
        // 外部点击：不变
        sw.event(&mut ctx, &UiEvent::MouseUp { pos: y901_ui::Pos::new(200.0, 200.0), button: y901_ui::MouseButton::Left });
        assert!(!sw.checked());
        // 内部点击：切换 + 回调
        sw.event(&mut ctx, &UiEvent::MouseUp { pos: y901_ui::Pos::new(20.0, 12.0), button: y901_ui::MouseButton::Left });
        assert!(sw.checked());
        assert_eq!(ctx.commands.len(), 1, "应发出一条命令");
    }
}