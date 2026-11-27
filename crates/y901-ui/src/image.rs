//! 图片绘制：纹理句柄、适配模式、着色。
//!
//! ## 为什么这几个类型放在 `y901-ui` 而不是组件库
//!
//! `RenderCtx` 要新增 `upload_texture` / `draw_image`，它们的参数类型必须和 trait 同层，
//! 否则后端（`y901-render`、`y901-backend-win32`）就得依赖组件库——依赖方向就倒了。

/// 纹理句柄。
///
/// 为什么用 newtype 而不是裸 `u64`：句柄来自**后端**（每次上传递增），
/// 裸 `u64` 很容易和"尺寸/坐标/颜色"混用，编译器却拦不住。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct TextureId(pub u64);

impl TextureId {
    /// 无效句柄（从未上传 / 上传失败）。绘制它应当什么都不画。
    pub const INVALID: TextureId = TextureId(0);

    /// 是否有效。
    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// 图片适配到目标矩形的模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// 拉伸填满（默认，忽略原始比例）
    #[default]
    Fill,
    /// 保持比例，完整可见（多余区域留白）
    Contain,
    /// 保持比例，填满目标（超出部分被裁掉）
    Cover,
    /// 原始尺寸，居中（不缩放）
    None,
}

/// 一次图片绘制的完整参数。
#[derive(Clone, Copy, Debug, Default)]
pub struct ImageStyle {
    /// 适配模式
    pub fit: ImageFit,
    /// 着色（`None` = 原色）。白色纹理 + 红色 tint = 纯红图标。
    pub tint: Option<crate::Color>,
    /// 额外裁剪区（与目标矩形取交集）
    pub clip: Option<crate::Rect>,
}

impl ImageStyle {
    /// 按适配模式算出"源图要画到目标矩形里的哪一块"。
    ///
    /// 输入：目标矩形、源图像素尺寸。
    /// 输出：`(目标子矩形, 源图子矩形)`，两者是**同一块区域**在两种坐标系里的表示。
    ///
    /// 纯函数、无副作用：后端只需按这两块做采样，不必各自重新推导一遍 fit 语义。
    pub fn layout(self, target: crate::Rect, src_w: u32, src_h: u32) -> (crate::Rect, crate::Rect) {
        let (sw, sh) = (src_w.max(1) as f32, src_h.max(1) as f32);
        let full_src = crate::Rect::new(0.0, 0.0, sw, sh);
        if target.w <= 0.0 || target.h <= 0.0 {
            return (crate::Rect::ZERO, crate::Rect::ZERO);
        }
        match self.fit {
            ImageFit::Fill => (target, full_src),
            ImageFit::Contain => {
                // 整张源图塞进去：取较小的缩放比
                let s = (target.w / sw).min(target.h / sh);
                let (w, h) = (sw * s, sh * s);
                let dst = crate::Rect::new(
                    target.x + (target.w - w) * 0.5,
                    target.y + (target.h - h) * 0.5,
                    w,
                    h,
                );
                (dst, full_src)
            }
            ImageFit::Cover => {
                // 铺满目标：取较大的缩放比，源图**裁掉**超出的部分
                let s = (target.w / sw).max(target.h / sh);
                let (vw, vh) = (target.w / s, target.h / s); // 可见的源图尺寸
                let src = crate::Rect::new((sw - vw) * 0.5, (sh - vh) * 0.5, vw, vh);
                (target, src)
            }
            ImageFit::None => {
                // 原始尺寸居中。源图比目标大时**两边同时裁**：
                //
                // 只把超出部分丢给调用方裁（旧行为）会得到"目标被裁、源仍是整张"的组合，
                // 采样时纹理会**拉伸**而不是裁切 —— 小图放进大框看不见错，大图放进小框
                // 就会明显变形。这里把交集算出来，反推出源图对应的子矩形。
                let dst_x = target.x + (target.w - sw) * 0.5;
                let dst_y = target.y + (target.h - sh) * 0.5;
                let visible = crate::Rect::new(dst_x, dst_y, sw, sh).intersect(target);
                if visible.w <= 0.0 || visible.h <= 0.0 {
                    return (crate::Rect::ZERO, crate::Rect::ZERO);
                }
                // 目标子矩形里的一像素 = 源图里的一像素（1:1），偏移量直接搬过去。
                let src =
                    crate::Rect::new(visible.x - dst_x, visible.y - dst_y, visible.w, visible.h);
                (visible, src)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn fill_stretches_to_target() {
        let (dst, src) = ImageStyle::default().layout(Rect::new(0.0, 0.0, 50.0, 50.0), 100, 50);
        assert_eq!(dst, Rect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(src, Rect::new(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn contain_letterboxes_and_keeps_aspect() {
        // 100x50 塞进 50x50：按宽度缩到 50x25，垂直居中
        let (dst, src) = ImageStyle {
            fit: ImageFit::Contain,
            ..Default::default()
        }
        .layout(Rect::new(0.0, 0.0, 50.0, 50.0), 100, 50);
        assert!(close(dst.w, 50.0) && close(dst.h, 25.0), "实际 {dst:?}");
        assert!(close(dst.y, 12.5), "应垂直居中，实际 y={}", dst.y);
        assert_eq!(src, Rect::new(0.0, 0.0, 100.0, 50.0), "源图不被裁");
    }

    #[test]
    fn cover_crops_the_source_horizontally() {
        // 100x50 铺满 50x50：按高度放大 1.0 → 可见宽度只有 50/1.0? 实际缩放 = max(0.5,1.0)=1.0
        let (dst, src) = ImageStyle {
            fit: ImageFit::Cover,
            ..Default::default()
        }
        .layout(Rect::new(0.0, 0.0, 50.0, 50.0), 100, 50);
        assert_eq!(dst, Rect::new(0.0, 0.0, 50.0, 50.0), "目标被填满");
        assert!(
            close(src.w, 50.0) && close(src.h, 50.0),
            "源图裁成 50x50，实际 {src:?}"
        );
        assert!(close(src.x, 25.0), "水平居中裁剪，实际 x={}", src.x);
    }

    #[test]
    fn none_keeps_native_size_centered() {
        let (dst, src) = ImageStyle {
            fit: ImageFit::None,
            ..Default::default()
        }
        .layout(Rect::new(0.0, 0.0, 200.0, 200.0), 40, 20);
        assert!(
            close(dst.x, 80.0) && close(dst.y, 90.0),
            "应居中，实际 {dst:?}"
        );
        assert!(close(dst.w, 40.0) && close(dst.h, 20.0), "原始尺寸");
        assert_eq!(src, Rect::new(0.0, 0.0, 40.0, 20.0));
    }

    /// **源图比目标大时两边同时裁**：旧实现只裁目标、源给整张 → 采样被拉伸变形。
    #[test]
    fn none_crops_both_sides_when_the_image_is_larger_than_the_target() {
        // 100×100 的图放进 40×40 的框：居中后可见的是源图 (30,30)-(70,70)。
        let (dst, src) = ImageStyle {
            fit: ImageFit::None,
            ..Default::default()
        }
        .layout(Rect::new(0.0, 0.0, 40.0, 40.0), 100, 100);
        assert_eq!(
            dst,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            "目标子矩形 = 整个目标框"
        );
        assert_eq!(
            src,
            Rect::new(30.0, 30.0, 40.0, 40.0),
            "源子矩形必须是对应的中间 40×40（1:1，不能是整张 100×100）"
        );
        // 关键不变量：两块面积相等 → 不会拉伸。
        assert!(
            close(dst.w * dst.h, src.w * src.h),
            "目标与源必须等面积（1:1）"
        );
    }

    /// 只在一个方向超出时，也只在那个方向裁。
    #[test]
    fn none_crops_only_the_overflowing_axis() {
        // 200×20 的图放进 100×100：水平居中裁左右，垂直完整可见。
        let (dst, src) = ImageStyle {
            fit: ImageFit::None,
            ..Default::default()
        }
        .layout(Rect::new(0.0, 0.0, 100.0, 100.0), 200, 20);
        assert_eq!(dst, Rect::new(0.0, 40.0, 100.0, 20.0));
        assert_eq!(src, Rect::new(50.0, 0.0, 100.0, 20.0));
        assert!(close(dst.h, 20.0), "高度不超，不该被裁");
    }

    #[test]
    fn empty_target_yields_empty_result() {
        let (dst, src) = ImageStyle::default().layout(Rect::ZERO, 10, 10);
        assert_eq!(dst, Rect::ZERO);
        assert_eq!(src, Rect::ZERO);
    }

    #[test]
    fn invalid_texture_id_is_not_valid() {
        assert!(!TextureId::INVALID.is_valid());
        assert!(TextureId(7).is_valid());
    }
}
