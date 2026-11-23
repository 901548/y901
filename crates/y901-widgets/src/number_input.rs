//! NumberInput：**可键盘编辑的数值输入框**（带步进、范围钳制与格式化）。
//!
//! ## 为什么单独做一个组件
//!
//! `TextInput` 是"任意文本"，而数值输入有它自己的规则：
//! 只能出现数字 / 一个小数点 / 前导负号；↑↓ 要步进；失焦或回车要**规范化**
//! （`"007"` → `7`、`"1."` → `1`、越界 → 夹到 min/max）。把这些塞进 `TextInput`
//! 会让两个组件的语义都变模糊。
//!
//! ## 交互约定
//!
//! | 输入 | 行为 |
//! |---|---|
//! | 数字 / `-` / `.` | 追加到编辑缓冲（越界字符直接忽略） |
//! | Backspace | 删一个字符 |
//! | ↑ / ↓ | 按 `step` 加减（`Shift` × 10） |
//! | Enter | 提交（解析 → 钳制 → 规范化 → 发命令） |
//! | Esc | 放弃编辑，恢复上一个已提交值 |
//!
//! 提交时发 `Command`，参数 `{value}`；`value` 是 `f64`（JSON 数字）。
//! 未提交的编辑不会被外部读到 —— `value()` 始终返回**已提交值**。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Constraints, EventCtx, EventPhase, EventResult, InputKind, Key, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 数值输入框。
pub struct NumberInput {
    id: String,
    key: u64,
    /// 已提交值。
    value: f64,
    /// 编辑缓冲（聚焦时用它渲染；`None` = 不在编辑态）。
    buffer: Option<String>,
    /// 是否持有焦点。
    focused: bool,
    /// 步进值（↑↓ 的粒度）。
    step: f64,
    /// 最小值。
    min: Option<f64>,
    /// 最大值。
    max: Option<f64>,
    /// 展示的小数位数（`0` = 整数）。
    decimals: usize,
    /// 提交时的命令。
    command: Option<y901_ui::Command>,
    /// 是否禁用。
    disabled: bool,
    rect: Rect,
    item: FlexItem,
    scale: f32,
}

impl NumberInput {
    /// 新建数值输入框（默认值 0，step 1，无范围，整数显示）。
    pub fn new(id: impl Into<String>, value: f64) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        // **非法初值不进入状态**：`NaN`/`inf` 一旦落进 `value`，格式化与钳制都会
        // 传下去（`clamp` 只比大小，不做有限性检查），最后是界面显示 `NaN`。
        let value = if value.is_finite() { value } else { 0.0 };
        Self {
            id,
            key,
            value,
            buffer: None,
            focused: false,
            step: 1.0,
            min: None,
            max: None,
            decimals: 0,
            command: None,
            disabled: false,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            scale: 1.0,
        }
    }

    /// 步进值（`↑`/`↓` 每次加减这么多；`Shift` 时 ×10）。
    pub fn step(mut self, step: f64) -> Self {
        if step.is_finite() && step > 0.0 {
            self.step = step;
        }
        self
    }

    /// 取值范围（提交时钳制到这里）。
    pub fn range(mut self, min: f64, max: f64) -> Self {
        if min.is_finite() && max.is_finite() && min <= max {
            self.min = Some(min);
            self.max = Some(max);
            // **钳制后再按 `decimals` 规范化**：否则 `value()` 会保留比任何提交路径
            // 都多的精度（显示 1.23、语义里是 1.23456），两边对不上。
            self.value = self.clamp(self.round(self.value));
        }
        self
    }

    /// 小数位数（`0..=6`；会用于展示与规范化的四舍五入）。
    pub fn decimals(mut self, places: usize) -> Self {
        self.decimals = places.min(6);
        // 改小数位后**立刻按新精度规范化**：`NumberInput::new("n", 1.23456).decimals(2)`
        // 旧实现里 `value()` 仍是 1.23456，而框里显示 1.23、`semantics` 也写 1.23。
        self.value = self.clamp(self.round(self.value));
        self
    }

    /// 提交时发出的命令（参数 `{value}`）。
    pub fn command(mut self, cmd: y901_ui::Command) -> Self {
        self.command = Some(cmd);
        self
    }

    /// 是否禁用（禁用后不接收事件，也不进 Tab 遍历）。
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 当前**已提交**值。
    pub fn value(&self) -> f64 {
        self.value
    }

    /// 以编程方式设置值（钳制 + 规范化；会触发重绘）。
    ///
    /// 返回：`true` = 值真的变了。
    pub fn set_value(&mut self, v: f64) -> bool {
        if !v.is_finite() {
            return false;
        }
        let v = self.clamp(self.round(v));
        if (self.value - v).abs() < f64::EPSILON {
            return false;
        }
        self.value = v;
        self.buffer = None;
        true
    }

    fn clamp(&self, v: f64) -> f64 {
        let mut v = v;
        if let Some(min) = self.min {
            v = v.max(min);
        }
        if let Some(max) = self.max {
            v = v.min(max);
        }
        v
    }

    fn round(&self, v: f64) -> f64 {
        let factor = 10f64.powi(self.decimals as i32);
        (v * factor).round() / factor
    }

    /// 展示用文本（编辑态用缓冲，否则用格式化后的值）。
    fn display_text(&self) -> String {
        match &self.buffer {
            Some(b) => b.clone(),
            None => format!("{:.*}", self.decimals, self.value),
        }
    }

    /// 解析编辑缓冲并提交。
    fn commit(&mut self, ctx: &mut EventCtx) -> bool {
        let Some(raw) = self.buffer.take() else {
            return false;
        };
        let trimmed = raw.trim();
        // 空串 / 只有符号 / 只有点：视为"没有输入"，恢复旧值，不改数据。
        let parsed = trimmed.parse::<f64>().ok().filter(|v| v.is_finite());
        let Some(v) = parsed else {
            return false;
        };
        let v = self.clamp(self.round(v));
        if (self.value - v).abs() < f64::EPSILON {
            return false;
        }
        self.value = v;
        if let Some(cmd) = self.command.as_ref() {
            ctx.emit_command(crate::command_with_args(
                cmd,
                y901_ui::json::Json::obj().set("value", v),
            ));
        }
        true
    }

    /// 在缓冲里追加一个字符（只接受数字 / 一个点 / 前导负号）。
    fn push_char(&mut self, ch: char) -> bool {
        let buf = self
            .buffer
            .get_or_insert_with(|| format!("{:.*}", self.decimals, self.value));
        match ch {
            '0'..='9' => buf.push(ch),
            '.' if self.decimals > 0 && !buf.contains('.') => buf.push('.'),
            '-' if buf.is_empty() => buf.push('-'),
            _ => return false,
        }
        true
    }

    /// 按步进加减（`Shift` × 10）。
    fn nudge(&mut self, ctx: &mut EventCtx, dir: f64, fast: bool) -> bool {
        // 编辑态先提交，避免"看着 A 值、步进从 B 值开始"。
        if self.buffer.is_some() {
            self.commit(ctx);
        }
        let delta = self.step * if fast { 10.0 } else { 1.0 } * dir;
        let next = self.clamp(self.round(self.value + delta));
        self.set_value(next)
    }
}

impl Widget for NumberInput {
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::TEXT_INPUT
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        let h = (m.line_height(&TextStyle::default()) + 8.0 * self.scale).max(28.0 * self.scale);
        c.constrain(Size::new(c.max.w, h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if self.disabled || ctx.phase == EventPhase::Capture {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::MouseDown { pos, .. } if self.rect.contains(*pos) => {
                ctx.request_focus(self.id.clone());
                self.focused = true;
                self.buffer = Some(format!("{:.*}", self.decimals, self.value));
                EventResult::Redraw
            }
            UiEvent::Focus(true) => {
                self.focused = true;
                EventResult::Ignored
            }
            // 失焦即提交：与桌面输入框一致（点别处等于确认），非法输入则丢弃。
            UiEvent::Focus(false) => {
                let had_buffer = self.buffer.is_some();
                self.focused = false;
                let changed = had_buffer && self.commit(ctx);
                if changed {
                    EventResult::Redraw
                } else {
                    self.buffer = None;
                    EventResult::Ignored
                }
            }
            UiEvent::Key { key, modifiers } => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                match key {
                    Key::Backspace => {
                        if let Some(buf) = self.buffer.as_mut() {
                            if buf.pop().is_some() {
                                return EventResult::Redraw;
                            }
                        }
                        EventResult::Handled
                    }
                    Key::Enter => {
                        if self.commit(ctx) {
                            EventResult::Redraw
                        } else {
                            self.buffer = None;
                            EventResult::Redraw
                        }
                    }
                    Key::Escape => {
                        // 放弃编辑：丢弃缓冲，回到已提交值。
                        self.buffer = None;
                        EventResult::Redraw
                    }
                    Key::Up | Key::Down => {
                        let dir = if matches!(key, Key::Up) { 1.0 } else { -1.0 };
                        if self.nudge(ctx, dir, modifiers.shift) {
                            EventResult::Redraw
                        } else {
                            EventResult::Handled
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            UiEvent::Text(text) => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                let mut changed = false;
                for ch in text.chars() {
                    changed |= self.push_char(ch);
                }
                if changed {
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Action { name, args } if name == "set_value" => {
                let v = match args.get("value").and_then(|v| v.as_f64()) {
                    Some(v) => v,
                    None => return EventResult::Ignored,
                };
                if self.set_value(v) {
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Action { name, args } if name == "step" => {
                let dir = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(1.0);
                if self.nudge(ctx, dir, false) {
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (bg, fg, border, muted) = {
            let theme = ctx.theme();
            let st = if self.disabled {
                State::Disabled
            } else if self.focused {
                State::Focused
            } else {
                State::Normal
            };
            let s = theme.style(Role::TEXT_INPUT, st);
            (s.bg, s.fg, s.fg, theme.muted_text_color())
        };
        ctx.fill_rect(self.rect, bg);
        ctx.stroke_round_rect(self.rect, border, 1.0, 0.0);

        let pad = 8.0 * self.scale;
        let text = self.display_text();
        let style = TextStyle::fg(if self.disabled { muted } else { fg });
        let line_h = ctx.line_height(&style);
        let ty = self.rect.y + (self.rect.h - line_h) * 0.5;
        ctx.push_clip(Rect::new(
            self.rect.x + pad,
            self.rect.y,
            (self.rect.w - pad * 2.0).max(0.0),
            self.rect.h,
        ));
        ctx.draw_text(Pos::new(self.rect.x + pad, ty), &text, &style);
        if self.focused {
            let w = ctx.measure_text(&text, &style).w;
            let caret_x = (self.rect.x + pad + w).min(self.rect.right() - pad);
            ctx.fill_rect(
                Rect::new(caret_x, ty, 1.0, line_h),
                if self.disabled { muted } else { fg },
            );
        }
        ctx.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(!self.disabled)
            .label(format!("number input {}", self.id))
            .value(self.display_text())
            .action_spec("set_value", InputKind::Number)
            .action_spec("step", InputKind::Number)
            .state("value", self.value)
            .state("focused", self.focused as u32)
            // 无障碍：范围与步进是一等字段（屏幕阅读器要念 "3 / 100"）。
            .value_step(self.step);
        if let (Some(min), Some(max)) = (self.min, self.max) {
            s = s.value_range(min, max);
        }
        if let Some(min) = self.min {
            s = s.prop("min", min.to_string());
        }
        if let Some(max) = self.max {
            s = s.prop("max", max.to_string());
        }
        s.prop("step", self.step.to_string())
            .prop("disabled", if self.disabled { "true" } else { "false" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use y901_ui::{Modifiers, Size, TextStyle};

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

    fn focus(w: &mut NumberInput, ctx: &mut EventCtx) {
        let mut cache = LayoutCache::new();
        w.measure(
            &Metrics,
            Constraints::loose(Size::new(120.0, 40.0)),
            &mut cache,
        );
        w.arrange(Rect::new(0.0, 0.0, 120.0, 32.0), &Metrics, &mut cache);
        w.event(
            ctx,
            &UiEvent::MouseDown {
                pos: Pos::new(5.0, 5.0),
                button: y901_ui::MouseButton::Left,
            },
        );
    }

    fn key(w: &mut NumberInput, ctx: &mut EventCtx, k: Key) {
        w.event(
            ctx,
            &UiEvent::Key {
                key: k,
                modifiers: Modifiers::default(),
            },
        );
    }

    #[test]
    fn starts_with_given_value_and_formats_decimals() {
        // **`value()` 必须等于显示与语义里的值**：旧实现保留原始精度
        // （`value() == 1.23456` 而框里显示 `1.23`），任何读取 `value()`
        // 的下游都会拿到一个界面上根本不存在、也不可能是提交结果的小数。
        let w = NumberInput::new("n", 1.23456).decimals(2);
        assert_eq!(w.value(), 1.23, "decimals(2) 之后 value() 应已规范化");
        assert_eq!(w.display_text(), "1.23");
        assert_eq!(
            format!("{}", w.value()),
            w.display_text(),
            "value() 与展示文本必须一致"
        );
        let w = NumberInput::new("n", 7.6).decimals(0);
        assert_eq!(w.display_text(), "8", "decimals(0) 展示时四舍五入");
        assert_eq!(w.value(), 8.0, "decimals(0) 之后 value() 也应取整");
    }

    #[test]
    fn range_normalizes_precision_too() {
        // range() 之前只 clamp 不 round：value() 会保留比任何提交路径都多的精度。
        let w = NumberInput::new("n", 1.23456).decimals(2).range(0.0, 10.0);
        assert_eq!(w.value(), 1.23);
        let w = NumberInput::new("n", 12.3456).decimals(1).range(0.0, 10.0);
        assert_eq!(w.value(), 10.0, "越界先夹到 max 再按小数位取整");
    }

    #[test]
    fn non_finite_initial_value_is_rejected() {
        let w = NumberInput::new("n", f64::NAN);
        assert_eq!(w.value(), 0.0, "NaN 初值不得进入状态");
        let w = NumberInput::new("n", f64::INFINITY);
        assert_eq!(w.value(), 0.0);
    }

    #[test]
    fn range_clamps_programmatic_and_keyboard_values() {
        let mut w = NumberInput::new("n", 5.0).range(0.0, 10.0);
        assert!(w.set_value(50.0), "越界值仍会改变状态（被夹到 max）");
        assert_eq!(w.value(), 10.0, "set_value 越界被夹到 max");
        assert!(!w.set_value(10.0), "同值写入返回 false");
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut w, &mut ctx);
        for _ in 0..20 {
            key(&mut w, &mut ctx, Key::Up);
        }
        assert_eq!(w.value(), 10.0, "连续 ↑ 不会越过 max");
    }

    #[test]
    fn typing_then_enter_commits_parsed_value() {
        let mut w = NumberInput::new("n", 0.0);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut w, &mut ctx);
        w.event(&mut ctx, &UiEvent::Text("42".to_string()));
        assert_eq!(w.value(), 0.0, "未提交前 value 不变");
        key(&mut w, &mut ctx, Key::Enter);
        assert_eq!(w.value(), 42.0, "Enter 提交解析值");
    }

    #[test]
    fn only_valid_characters_are_accepted() {
        let mut w = NumberInput::new("n", 0.0).decimals(1);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut w, &mut ctx);
        // 先清空缓冲（默认缓冲是 "0.0"）
        for _ in 0..3 {
            key(&mut w, &mut ctx, Key::Backspace);
        }
        w.event(&mut ctx, &UiEvent::Text("abc1.5.".to_string()));
        let buf = w.buffer.clone().unwrap();
        assert_eq!(buf, "1.5", "字母与第二个小数点被忽略");
        // 整数模式不接受小数点
        let mut int = NumberInput::new("i", 0.0);
        let mut ctx2 = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut int, &mut ctx2);
        for _ in 0..1 {
            key(&mut int, &mut ctx2, Key::Backspace);
        }
        int.event(&mut ctx2, &UiEvent::Text("1.5".to_string()));
        assert_eq!(int.buffer.clone().unwrap(), "15", "小数点为 0 时不接受点");
    }

    #[test]
    fn escape_restores_committed_value() {
        let mut w = NumberInput::new("n", 12.0);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut w, &mut ctx);
        for _ in 0..2 {
            key(&mut w, &mut ctx, Key::Backspace);
        }
        w.event(&mut ctx, &UiEvent::Text("99".to_string()));
        key(&mut w, &mut ctx, Key::Escape);
        assert_eq!(w.value(), 12.0);
        assert_eq!(w.display_text(), "12");
    }

    #[test]
    fn shift_arrow_uses_ten_times_step() {
        let mut w = NumberInput::new("n", 0.0).step(2.0).range(-100.0, 100.0);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut w, &mut ctx);
        key(&mut w, &mut ctx, Key::Up);
        assert_eq!(w.value(), 2.0);
        w.event(
            &mut ctx,
            &UiEvent::Key {
                key: Key::Up,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
        );
        assert_eq!(w.value(), 22.0, "Shift+↑ = step × 10");
    }

    #[test]
    fn disabled_ignores_input() {
        let mut w = NumberInput::new("n", 1.0).disabled(true);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
        focus(&mut w, &mut ctx);
        key(&mut w, &mut ctx, Key::Up);
        assert_eq!(w.value(), 1.0, "禁用时按键无效");
        assert!(!w.semantics().exposed_to_ai, "禁用时不对外部暴露");
    }

    #[test]
    fn semantics_exposes_range_and_value() {
        let w = NumberInput::new("n", 5.0).range(1.0, 9.0).step(0.5);
        let s = w.semantics();
        assert!(s.exposed_to_ai);
        let props: Vec<_> = s
            .props
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(props.contains(&("min", "1")));
        assert!(props.contains(&("max", "9")));
        assert!(props.contains(&("step", "0.5")));
        let acts: Vec<_> = s.actions.iter().map(|a| a.name.as_str()).collect();
        assert!(acts.contains(&"set_value"));
        assert!(acts.contains(&"step"));
    }
}
