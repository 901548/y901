//! Slider：水平轨道 + 圆形滑块。
//!
//! 交互：点击轨道跳转、按住滑块拖动、左右键微调（步进 0.05）。
//! 滑块中心允许超出轨道两端半个滑块宽度 —— 这是滑块的常规观感（0 与 1 位置更易命中）。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Key, Modifiers, Rect, RenderCtx, Role, Semantics,
    Size, State, TextMetrics, UiEvent, Widget,
};

/// 轨道高度。
const TRACK_H: f32 = 4.0;
/// 把任意输入收敛到 `0..=1`，**非法值（NaN / ±∞）归 0**。
///
/// 为什么不能只用 `clamp`：`f32::clamp` 对 NaN 返回 NaN（IEEE 语义），
/// 于是 NaN 会一路流进布局与提交，最后变成整帧不可见的黑屏。
fn sane(v: f32) -> f32 {
    if v.is_nan() {
        // NaN 没有"大小"可言，只能退回 0
        0.0
    } else {
        // ±∞ 本身有方向，clamp 会正确归到 0 或 1
        v.clamp(0.0, 1.0)
    }
}

/// 滑块直径。
const THUMB: f32 = 16.0;
/// 键盘步进。
const STEP: f32 = 0.05;
/// 命中滑块的水平容差（半径 + 2px）。
// 真死代码：拖拽用整个轨道的 rect 判命中，不需要“带宽度的提帝区”；
// 保留它仅为 `hits_thumb` 的测试可读性，下一轮一并删除。
// 归类：内部常量 —— `HIT_R` 与 `hits_thumb` 成对，仅后者在用时被内联
#[allow(dead_code)] // 预留 API：当前未接线，留作未来拆分/扩展使用
const HIT_R: f32 = THUMB * 0.5 + 2.0;

// ===================== DPI 缩放（下面三个函数）=====================
//
// **为何尺寸要随 scale**：滑块与轨道是**触控/鼠标目标**，
// 125% DPI 下文字与其他控件都放大 25%，而 16px 的滑块不变 → 它会变得难点。
//
// **为何 `2.0` 不乘 scale**：那是**命中容差**（手指/鼠标精度补偿），
// 属于**输入设备属性**，不是视觉尺寸；放大它会让相邻控件命中区重叠。
// `Slider` 没有 `&self` 公开几何 API（`track`/`thumb`/`value_at`/`hits_thumb` 均私有）→ **不加缓存**。

/// 轨道厚度（逻辑像素 × DPI 缩放）。
fn track_h(scale: f32) -> f32 {
    TRACK_H * scale
}

/// 滑块边长（逻辑像素 × DPI 缩放）。
fn thumb_size(scale: f32) -> f32 {
    THUMB * scale
}

/// 滑块命中半径：**随滑块变大，但固定容差 2px 不变**。
fn hit_r(scale: f32) -> f32 {
    thumb_size(scale) * 0.5 + 2.0
}

/// 水平滑块。
pub struct Slider {
    /// 无障碍名称（读屏器念它）。空 = 不提供名称。
    label: String,
    id: String,
    value: f32,
    on_change: Option<Box<dyn Fn(f32)>>,
    rect: Rect,
    focused: bool,
    dragging: bool,
}

impl Slider {
    /// 创建滑块。`value` 会被 clamp 到 `0..=1`。
    pub fn new(id: impl Into<String>, value: f32) -> Self {
        Self {
            label: String::new(),
            id: id.into(),
            // clamp 对 NaN 返回 NaN（IEEE 语义），必须先排查非法值
            value: sane(value),
            on_change: None,
            rect: Rect::ZERO,
            focused: false,
            dragging: false,
        }
    }

    /// 无障碍名称（读屏器念的名字）。见 `Switch::label` 的说明。
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// 值变化回调。
    pub fn on_change(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// 当前值（0..=1）。
    pub fn value(&self) -> f32 {
        self.value
    }

    /// 设置值（clamp + 触发回调，仅当真的变化）。
    pub fn set_value(&mut self, v: f32) {
        let v = sane(v);
        if (v - self.value).abs() > f32::EPSILON {
            self.value = v;
            if let Some(cb) = self.on_change.as_ref() {
                cb(v);
            }
        }
    }

    /// 轨道矩形（水平铺满、垂直居中）。
    fn track(&self, scale: f32) -> Rect {
        let h = track_h(scale).min(self.rect.h);
        Rect::new(
            self.rect.x,
            self.rect.y + (self.rect.h - h) * 0.5,
            self.rect.w.max(0.0),
            h,
        )
    }

    /// 滑块中心 x。
    fn thumb_center_x(&self, _scale: f32) -> f32 {
        // 滑块中心只取决于 rect 与 value（均为物理像素）——
        // 保留参数为了让三个方法链同一形状，避免调用方记得何处要传何处不用。
        self.rect.x + self.rect.w * self.value
    }

    /// 滑块矩形（正方形，垂直居中）。
    fn thumb(&self, scale: f32) -> Rect {
        let cx = self.thumb_center_x(scale);
        let t = thumb_size(scale);
        Rect::new(cx - t * 0.5, self.rect.y + (self.rect.h - t) * 0.5, t, t)
    }

    /// 把屏幕 x 换算成值（clamp 到 0..=1）。
    fn value_at(&self, x: f32, _scale: f32) -> f32 {
        if self.rect.w <= 0.0 {
            return self.value;
        }
        ((x - self.rect.x) / self.rect.w).clamp(0.0, 1.0)
    }

    /// 指针是否命中滑块。
    ///
    /// 目前只在测试里用：鼠标按下时整个矩形都算命中（点轨道也要跳转），
    /// 但"点滑块只跟随拖拽、点轨道直接跳"这种更细的手感分支需要它。
    // 测试 API：仅 slider 单元测试调用，作为“命中判定语义”的可执行文档。
    // 归类：内部实现 —— `hits_thumb` 已被等价逻辑取代，保留作对照
    #[allow(dead_code)] // 预留 API：当前未接线，留作未来拆分/扩展使用
    fn hits_thumb(&self, x: f32, y: f32, scale: f32) -> bool {
        let t = self.thumb(scale);
        // **固定容差 2.0 与 `HIT_R` 里的 `+2.0` 是同一含义**（输入精度补偿），
        // 统一走 `hit_r(scale)`：它 = 滑块半径（随 scale）+ 2px（不随 scale）。
        // 水平方向的 `±t.h*0.5`已由 `hit_r` 的 `thumb_size*0.5` 覆盖，所以水平也用它。
        let r = hit_r(scale);
        x >= t.x - 2.0 && x <= t.right() + 2.0 && (y - (t.y + t.h * 0.5)).abs() <= r
    }
}

impl Widget for Slider {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        // **语义角色**：读屏器念"滑杆" + 当前值 + 范围。配色仍走 button 槽（见 `Theme::slot`）。
        Role::SLIDER
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
        c.constrain(Size::new(c.max.w, thumb_size(m.scale())))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseDown { pos, .. } => {
                if !self.rect.contains(*pos) {
                    return EventResult::Ignored;
                }
                ctx.request_focus(self.id.clone());
                // 点在滑块上 → 拖动模式；点在轨道上 → 直接跳转并进入拖动
                // （跳转后再拖是滑块的常见手感，用户不必再按一次）
                self.dragging = true;
                ctx.set_pointer_capture(self.id.clone());
                self.set_value(self.value_at(pos.x, ctx.scale()));
                // **值变了就要重绘**：`EventResult::Handled < Redraw`，
                // 只回 `Handled` 时框架不会标脏 —— 点击后滑块停在旧位置，
                // 要等下一次无关事件才跳过去。
                EventResult::Redraw
            }
            UiEvent::MouseMove { pos } => {
                if !self.dragging {
                    return EventResult::Ignored;
                }
                self.set_value(self.value_at(pos.x, ctx.scale()));
                EventResult::Redraw
            }
            UiEvent::MouseUp { .. } => {
                if self.dragging {
                    self.dragging = false;
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            UiEvent::Focus(focused) => {
                self.focused = *focused;
                EventResult::Redraw
            }
            UiEvent::Key { key, modifiers } => {
                if *modifiers != Modifiers::default() {
                    return EventResult::Ignored;
                }
                match key {
                    Key::Left => {
                        let before = self.value;
                        self.set_value(self.value - STEP);
                        if self.value != before {
                            EventResult::Redraw
                        } else {
                            EventResult::Handled
                        }
                    }
                    Key::Right => {
                        let before = self.value;
                        self.set_value(self.value + STEP);
                        if self.value != before {
                            EventResult::Redraw
                        } else {
                            EventResult::Handled
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let normal = ctx.theme().style(Role::BUTTON, State::Normal);
        let accent = ctx.theme().style(Role::BUTTON, State::Selected);
        let track = self.track(ctx.scale());
        if track.w <= 0.0 {
            return;
        }

        // 轨道
        ctx.fill_round_rect(track, normal.bg, track_h(ctx.scale()) * 0.5);
        // 已填充部分（从左侧到滑块中心）
        let filled = (self.thumb_center_x(ctx.scale()) - track.x).clamp(0.0, track.w);
        if filled > 0.0 {
            ctx.fill_round_rect(
                Rect::new(track.x, track.y, filled, track.h),
                accent.bg,
                track_h(ctx.scale()) * 0.5,
            );
        }

        // 滑块：外圈边框 + 内圆，焦点时边框用焦点色
        let thumb = self.thumb(ctx.scale());
        let border = if self.focused || self.dragging {
            ctx.theme().style(Role::BUTTON, State::Focused).fg
        } else {
            accent.bg
        };
        ctx.fill_round_rect(thumb, border, thumb_size(ctx.scale()) * 0.5);
        // 内圈内缩 2px 是**描边宽度**（不随 scale），所以口径是 `t - 4.0`。
        let t = thumb_size(ctx.scale());
        let inner = Rect::new(thumb.x + 2.0, thumb.y + 2.0, t - 4.0, t - 4.0);
        ctx.fill_round_rect(inner, normal.bg, (t - 4.0) * 0.5);
        let _ = Color::WHITE;
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            // 不再把内部 id 拼进名字（见 `Switch::label` 的说明）。
            .label(self.label.clone())
            .value(format!("{:.2}", self.value))
            .action("set")
            .prop("value", format!("{:.2}", self.value))
            // 滑杆的 value 是 0..=1 的比例，因此范围固定，步进取 0.01。
            .value_range(0.0, 1.0)
            .value_step(0.01)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;
    use y901_ui::layout::LayoutCache;

    const W: u32 = 200;
    const H: u32 = 40;

    fn soft() -> SoftBackend {
        let (cw, ch) = y901_render::cell_metrics_for(15.0);
        let cell = Size::new(cw, ch);
        let engine = MyFontEngine::new(
            y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
            15.0,
            cell,
        )
        .expect("engine");
        SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
    }

    fn render(value: f32) -> (SoftBackend, Slider) {
        let mut s = soft();
        s.clear();
        let mut sl = Slider::new("s", value);
        let mut cache = LayoutCache::new();
        sl.arrange(Rect::new(10.0, 10.0, 100.0, 20.0), &s, &mut cache);
        sl.render(&mut s);
        (s, sl)
    }

    /// 找到"最亮的一列"，返回其中心 x（滑块是画面中最亮的块）。
    fn brightest_column(s: &SoftBackend) -> (u32, u8) {
        let mut best = (0u32, 0u8);
        for x in 0..W {
            let mut sum = 0u32;
            for y in 0..H {
                let p = s.pixel(x, y).expect("像素");
                let r = (p >> 16) & 0xFF;
                let g = (p >> 8) & 0xFF;
                let b = p & 0xFF;
                sum += (r + g + b) / 3;
            }
            let avg = (sum / H) as u8;
            if avg > best.1 {
                best = (x, avg);
            }
        }
        best
    }

    #[test]
    fn thumb_center_tracks_the_value() {
        for (value, want_x) in [(0.0_f32, 10.0_f32), (0.5, 60.0), (1.0, 110.0)] {
            let (s, sl) = render(value);
            let (bx, _) = brightest_column(&s);
            eprintln!(
                "[slider] value={value} 最亮列 = {bx}，滑块中心应为 {want_x}（实测中心 {:.1}）",
                sl.thumb_center_x(1.0)
            );
            assert!(
                (sl.thumb_center_x(1.0) - want_x).abs() < 0.5,
                "value={value} 时滑块中心应为 {want_x}"
            );
            assert!(
                (bx as f32 - want_x).abs() <= 3.0,
                "value={value} 时画面最亮列应接近 {want_x}，实际 {bx}"
            );
        }
    }

    #[test]
    fn click_maps_position_to_value() {
        let mut sl = Slider::new("s", 0.0);
        sl.rect = Rect::new(0.0, 0.0, 100.0, 20.0);
        // 75% 处
        let v = sl.value_at(75.0, 1.0);
        assert!((v - 0.75).abs() < 1e-4, "75% 位置应映射到 0.75，实际 {v}");
        assert_eq!(sl.value_at(-10.0, 1.0), 0.0, "左侧越界应 clamp 到 0");
        assert_eq!(sl.value_at(200.0, 1.0), 1.0, "右侧越界应 clamp 到 1");
    }

    #[test]
    fn value_setter_clamps_and_notifies_once_per_change() {
        use std::cell::Cell;
        use std::rc::Rc;
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let mut sl = Slider::new("s", 0.2).on_change(move |_| h.set(h.get() + 1));

        sl.set_value(0.5);
        assert!((sl.value() - 0.5).abs() < 1e-6);
        assert_eq!(hits.get(), 1);

        sl.set_value(0.5); // 同值 → 不回调
        assert_eq!(hits.get(), 1, "值未变化时不应触发回调");

        sl.set_value(2.0); // clamp 到 1.0 → 算一次变化
        assert_eq!(sl.value(), 1.0);
        assert_eq!(hits.get(), 2);

        sl.set_value(-1.0);
        assert_eq!(sl.value(), 0.0);
        assert_eq!(hits.get(), 3);
    }

    #[test]
    fn thumb_hit_test_covers_the_knob() {
        let mut sl = Slider::new("s", 0.5);
        sl.rect = Rect::new(0.0, 0.0, 100.0, 20.0);
        let cx = sl.thumb_center_x(1.0);
        assert!(sl.hits_thumb(cx, 10.0, 1.0), "滑块中心应命中");
        assert!(sl.hits_thumb(cx + 6.0, 10.0, 1.0), "滑块右缘附近应命中");
        assert!(!sl.hits_thumb(cx + 40.0, 10.0, 1.0), "远离滑块不应命中");
    }
}
