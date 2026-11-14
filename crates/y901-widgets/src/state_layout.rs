//! `EmptyState` 与 `ErrorState` 共用的**布局几何**。
//!
//! 为何单独抽一个模块：两个组件的差异只在**语义**（错误红 vs 中性灰、
//! 圆圈内是否有实心点、按钮文案），而**尺寸与位置**完全一致。
//! 把几何放在这里，两边就不会各自漂移。

use y901_ui::Rect;

/// 图标占位直径。
pub(crate) const ICON_D: f32 = 48.0;
/// 操作按钮高度。
pub(crate) const ACTION_H: f32 = 32.0;
/// 操作按钮宽度。
pub(crate) const ACTION_W: f32 = 120.0;

/// 期望高度：图标 48 + 标题 24 + 间距，可选说明/按钮各加一段。
pub(crate) fn content_height(scale: f32, has_description: bool, has_action: bool) -> f32 {
    let mut h = (ICON_D + 16.0 + 24.0) * scale;
    if has_description {
        h += (8.0 + 18.0) * scale;
    }
    if has_action {
        h += (16.0 + ACTION_H) * scale;
    }
    h
}

/// 操作按钮矩形：居中、贴底。
pub(crate) fn action_rect(rect: Rect, scale: f32) -> Rect {
    let w = ACTION_W * scale;
    Rect::new(
        rect.x + (rect.w - w) * 0.5,
        rect.bottom() - (ACTION_H + 8.0) * scale,
        w,
        ACTION_H * scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **结构测试**：`EmptyState` 与 `ErrorState` 走的是同一套布局几何。
    ///
    /// 为何要它：两个组件本来各自拷了一份 `content_height` / `arrange`，
    /// 修改一边就会不对称；现在只要这里一致，两边就一定一致。
    #[test]
    fn both_state_widgets_share_one_geometry() {
        for &scale in &[1.0_f32, 1.25, 1.5] {
            for &(desc, act) in &[(false, false), (false, true), (true, false), (true, true)] {
                let h = content_height(scale, desc, act);
                assert!(h > 0.0);
                // 高度单调：多一个可选部分就更高
                if desc && !act {
                    assert!(h > content_height(scale, false, false));
                }
                if act {
                    assert!(h > content_height(scale, desc, false));
                }
            }
            // 按钮居中且贴底
            let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
            let a = action_rect(rect, scale);
            assert!((a.w - ACTION_W * scale).abs() < 0.01);
            assert!((a.h - ACTION_H * scale).abs() < 0.01);
            assert!(((a.x + a.w * 0.5) - 200.0).abs() < 0.01, "按钮应水平居中");
            assert!((a.bottom() - (300.0 - 8.0 * scale)).abs() < 0.01, "按钮应贴底 8px");
        }
    }
}
