//! 两个渲染后端共享的渲染状态：裁剪栈、变换栈、悬停、主题、动画快照。
//!
//! # 为什么有这个模块
//!
//! `SoftBackend`（CPU）与 `D3dBackend`（D3D11）原本各自维护**一模一样**的五个字段
//! （`clips` / `transforms` / `hovered` / `theme` / `anims`），
//! 并且各自写了一份 `push_clip` / `pop_clip` / `push_transform` / `pop_transform`
//! / `set_hovered` / `is_hovered` / `set_animations` / `animation` / `background_color`。
//!
//! 本模块把这五个字段与它们的纯逻辑收拢到一处，两个后端各持有一个实例。
//!
//! # 故意**不**包含的东西
//!
//! - `map_rect` / `clip_of` / `clip_rect`：平台差异（一个用 `map_point` 推矩形、一个逐点映射），留在各自后端。
//! - 任何平台类型（`HWND` / X11 连接）：本模块只依赖 `y901-ui` 的纯数据类型。
//!
//! # 已知不对称（不要强行统一）
//!
//! `current_scale()` 目前**只有 D3D 后端**在用（`D3dBackend::current_scale`）；
//! Soft 在需要缩放时是从 `map_rect(1x1).w` 反推的。两者在**均匀缩放**下等价，
//! 但语义来源不同，所以本模块只提供"变换栈缩放的乘积"这一个定义，
//! **是否使用由后端自己决定**。

use y901_ui::animation::AnimSnapshot;
use y901_ui::theme::Theme;
use y901_ui::{Color, Pos, Rect, Role};

/// 两个后端共享的渲染状态。
///
/// 全部字段私有：读写都走方法，保证两个后端的行为一致。
pub struct RenderState {
    clips: Vec<Rect>,
    transforms: Vec<(Pos, f32)>,
    hovered: Option<String>,
    theme: Theme,
    anims: AnimSnapshot,
    /// 不透明度栈：每一层的值乘积就是当前不透明度（嵌套语义）。
    ///
    /// 为何是栈而不是单字段：`push(0.5)` 两次必须得到 0.25，
    /// 单字段只能表示一层，写成赋值会丢掉外层。
    opacity_stack: Vec<f32>,
}

impl RenderState {
    /// 以给定主题创建空状态（裁剪栈与变换栈为空，无悬停，动画快照为默认）。
    pub fn new(theme: Theme) -> Self {
        Self {
            clips: Vec::new(),
            transforms: Vec::new(),
            hovered: None,
            theme,
            anims: AnimSnapshot::default(),
            opacity_stack: Vec::new(),
        }
    }

    // ---- 裁剪栈 ----

    /// 压入一个裁剪矩形（与栈顶求交由调用方决定，保持与既有后端一致：这里只入栈）。
    pub fn push_clip(&mut self, rect: Rect) {
        self.clips.push(rect);
    }

    /// 弹出栈顶裁剪矩形；空栈时是无操作（两个后端的原实现都是 `pop`，空栈不会 panic）。
    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    /// 整个裁剪栈（用于诊断与测试）。
    pub fn clips(&self) -> &[Rect] {
        &self.clips
    }

    /// 栈顶裁剪矩形；空栈返回 `None`。
    pub fn current_clip(&self) -> Option<Rect> {
        self.clips.last().copied()
    }

    /// 清空裁剪栈。
    pub fn clear_clips(&mut self) {
        self.clips.clear();
    }

    // ---- 变换栈 ----

    /// 压入一组变换（原点 + 缩放）。
    pub fn push_transform(&mut self, origin: Pos, scale: f32) {
        self.transforms.push((origin, scale));
    }

    /// 弹出栈顶变换；空栈时是无操作。
    pub fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    /// 整个变换栈。
    pub fn transforms(&self) -> &[(Pos, f32)] {
        &self.transforms
    }

    /// 变换栈是否为空（供后端做快路径判断）。
    pub fn transforms_empty(&self) -> bool {
        self.transforms.is_empty()
    }

    /// 多级变换缩放的**乘积**；空栈或非法值返回 `1.0`。
    ///
    /// 与 `D3dBackend::current_scale` 的算法一致（非有限或 ≤ 0 时回退 1.0）。
    pub fn current_scale(&self) -> f32 {
        if self.transforms.is_empty() {
            return 1.0;
        }
        let k: f32 = self.transforms.iter().map(|(_, s)| *s).product();
        if k.is_finite() && k > 0.0 {
            k
        } else {
            1.0
        }
    }

    // ---- 悬停 ----

    /// 设置当前悬停节点 id（`None` = 无悬停）。
    pub fn set_hovered(&mut self, id: Option<&str>) {
        self.hovered = id.map(|s| s.to_string());
    }

    /// 某节点是否正被悬停。
    pub fn is_hovered(&self, id: &str) -> bool {
        self.hovered.as_deref() == Some(id)
    }

    // ---- 主题 ----

    /// 替换主题。
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// 当前主题。
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// 面板背景色（Tabs 过渡遮罩等处使用）。
    pub fn background_color(&self) -> Color {
        self.theme
            .style(Role::PANEL, y901_ui::theme::State::Normal)
            .bg
    }

    // ---- 动画 ----

    /// 推入本帧动画快照。
    pub fn set_animations(&mut self, snapshot: &AnimSnapshot) {
        self.anims = snapshot.clone();
    }

    /// 查询动画进度；键不存在时回退 `1.0`（即"已完成"）。
    pub fn animation(&self, key: u64) -> f32 {
        self.anims.get(key).unwrap_or(1.0)
    }

    // ---- 不透明度 ----

    /// 压入一层不透明度（自动 clamp 到 `[0, 1]`）。
    ///
    /// 嵌套时是**乘积**：push(0.5) 两次 → `current_opacity() == 0.25`。
    pub fn push_opacity(&mut self, opacity: f32) {
        self.opacity_stack.push(opacity.clamp(0.0, 1.0));
    }

    /// 弹出栈顶不透明度；空栈时无操作。
    pub fn pop_opacity(&mut self) {
        self.opacity_stack.pop();
    }

    /// 当前不透明度 = 栈中所有值的乘积；空栈为 `1.0`。
    pub fn current_opacity(&self) -> f32 {
        self.opacity_stack.iter().product::<f32>().clamp(0.0, 1.0)
    }

    /// 不透明度栈深度（诊断用）。
    pub fn opacity_depth(&self) -> usize {
        self.opacity_stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::theme::LIGHT;

    fn state() -> RenderState {
        RenderState::new(Theme::DARK)
    }

    #[test]
    fn clip_stack_is_lifo_and_empty_safe() {
        let mut s = state();
        assert_eq!(s.current_clip(), None);
        s.pop_clip(); // 空栈不 panic
        let a = Rect::new(1.0, 2.0, 3.0, 4.0);
        let b = Rect::new(5.0, 6.0, 7.0, 8.0);
        s.push_clip(a);
        s.push_clip(b);
        assert_eq!(s.current_clip(), Some(b));
        s.pop_clip();
        assert_eq!(s.current_clip(), Some(a));
        s.clear_clips();
        assert!(s.clips().is_empty());
    }

    #[test]
    fn current_scale_is_product_of_push_and_falls_back_to_one() {
        let mut s = state();
        assert_eq!(s.current_scale(), 1.0, "空栈应为 1.0");
        s.push_transform(Pos::new(0.0, 0.0), 2.0);
        s.push_transform(Pos::new(0.0, 0.0), 1.5);
        assert!((s.current_scale() - 3.0).abs() < 1e-6, "应为乘积 3.0");
        s.pop_transform();
        assert!((s.current_scale() - 2.0).abs() < 1e-6);
        // 非法缩放（0）应回退 1.0，而不是返回 0（否则后续会画出 0 尺寸）
        s.push_transform(Pos::new(0.0, 0.0), 0.0);
        assert_eq!(s.current_scale(), 1.0);
    }

    #[test]
    fn hovered_matches_exact_id_only() {
        let mut s = state();
        assert!(!s.is_hovered("a"));
        s.set_hovered(Some("a"));
        assert!(s.is_hovered("a"));
        assert!(!s.is_hovered("ab"), "悬停必须整串相等，不能前缀匹配");
        s.set_hovered(None);
        assert!(!s.is_hovered("a"));
    }

    #[test]
    fn animation_falls_back_to_one_for_unknown_keys() {
        let s = state();
        assert_eq!(s.animation(999), 1.0);
    }

    #[test]
    fn background_color_follows_theme() {
        let mut s = state();
        let dark = s.background_color();
        s.set_theme(LIGHT);
        let light = s.background_color();
        assert_ne!(dark, light, "换主题后背景色必须变化");
    }

    #[test]
    fn opacity_is_a_multiplicative_stack() {
        let mut s = state();
        assert_eq!(s.current_opacity(), 1.0, "空栈为 1.0");
        s.push_opacity(0.5);
        assert_eq!(s.current_opacity(), 0.5);
        s.push_opacity(0.5);
        assert!((s.current_opacity() - 0.25).abs() < 1e-6, "嵌套应乘积");
        s.pop_opacity();
        assert_eq!(s.current_opacity(), 0.5);
        s.pop_opacity();
        assert_eq!(s.current_opacity(), 1.0);
        s.pop_opacity(); // 空栈不 panic
        assert_eq!(s.opacity_depth(), 0);
    }

    #[test]
    fn opacity_is_clamped_to_unit_range() {
        let mut s = state();
        s.push_opacity(-1.0);
        assert_eq!(s.current_opacity(), 0.0);
        s.pop_opacity();
        s.push_opacity(2.0);
        assert_eq!(s.current_opacity(), 1.0);
    }
}
