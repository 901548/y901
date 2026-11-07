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
                // 原始尺寸居中；超出目标的部分由调用方裁剪
                let dst = crate::Rect::new(
                    target.x + (target.w - sw) * 0.5,
                    target.y + (target.h - sh) * 0.5,
                    sw,
                    sh,
                );
                (dst, full_src)
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
