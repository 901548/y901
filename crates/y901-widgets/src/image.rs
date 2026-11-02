//! Image 控件：持有一张 RGBA 位图，首次渲染时上传为纹理并绘制。

use y901_ui::image::{ImageFit, ImageStyle, TextureId};
use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, UiEvent,
    Widget,
};

/// 图片控件。
pub struct Image {
    id: String,
    /// 位图宽度
    width: u32,
    /// 位图高度
    height: u32,
    /// RGBA8（行优先，无 padding）
    rgba: Vec<u8>,
    /// 绘制参数
    style: ImageStyle,
    /// 惰性上传后的纹理句柄（`None` = 还没上传）
    texture: Option<TextureId>,
    /// 布局矩形
    rect: Rect,
    /// 在 Flex 中的尺寸约束
    item: y901_ui::FlexItem,
}

impl Image {

    /// 设置在 Flex 中的尺寸约束（链式）。
    pub fn flex_item(mut self, item: y901_ui::FlexItem) -> Self {
        self.item = item;
        self
    }
    /// 从原始 RGBA 数据创建。
    ///
    /// 输入：`id`、尺寸、`rgba`（长度必须 ≥ `w*h*4`）。
    /// 行为：长度不足时**截断为黑底透明**，不 panic —— 调用方给错数据不该炸掉 UI。
    pub fn from_rgba(id: impl Into<String>, w: u32, h: u32, rgba: &[u8]) -> Self {
        let need = (w as usize) * (h as usize) * 4;
        let mut data = Vec::with_capacity(need);
        data.extend_from_slice(&rgba[..rgba.len().min(need)]);
        data.resize(need, 0);
        Self {
            id: id.into(),
            width: w,
            height: h,
            rgba: data,
            style: ImageStyle::default(),
            texture: None,
            rect: Rect::ZERO,
            item: y901_ui::FlexItem::default(),
        }
    }

    /// 从 PNG 字节解码创建。
    ///
    /// 返回值：`Err` 带可读原因（解码失败/不支持的颜色类型）。
    ///
    /// 为什么用 `png` crate 而不是手写解码：它只依赖 `flate2`→`miniz_oxide`，
    /// **全链纯 Rust**，零 C 依赖守卫测试会替我们继续盯着这一点。
    pub fn from_png(id: impl Into<String>, bytes: &[u8]) -> Result<Self, String> {
        let decoder = png::Decoder::new(bytes);
        let mut reader = decoder.read_info().map_err(|e| format!("PNG 读取失败: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("PNG 解码失败: {e}"))?;
        let (w, h) = (info.width, info.height);
        // 统一转 RGBA8：灰度/调色板/16 位都由 png crate 负责展开
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf[..(w as usize * h as usize * 4)].to_vec(),
            png::ColorType::Rgb => {
                let mut out = Vec::with_capacity(w as usize * h as usize * 4);
                for px in buf[..(w as usize * h as usize * 3)].chunks_exact(3) {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
                out
            }
            other => return Err(format!("暂不支持的 PNG 颜色类型: {other:?}")),
        };
        Ok(Self::from_rgba(id, w, h, &rgba))
    }

    /// 设置绘制参数（适配模式、着色、裁剪）。
    pub fn with_style(mut self, style: ImageStyle) -> Self {
        self.style = style;
        self
    }

    /// **仅测试用**：收养一张已有纹理（不上传自己的位图）。
    #[doc(hidden)]
    pub fn adopt_texture_for_test(&mut self, id: y901_ui::image::TextureId) {
        self.texture = Some(id);
    }

    /// 位图尺寸。
    pub fn image_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 便捷：指定适配模式。
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.style.fit = fit;
        self
    }

    /// 便捷：指定着色。
    pub fn tint(mut self, color: y901_ui::Color) -> Self {
        self.style.tint = Some(color);
        self
    }
}

impl Widget for Image {
    fn id(&self) -> &str {
        &self.id
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 按原始像素尺寸申请空间，再由约束裁剪；尺寸未知时退回约束上限
        let want = Size::new(self.width.max(1) as f32, self.height.max(1) as f32);
        c.constrain(want)
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn flex(&self) -> y901_ui::FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        // 惰性上传：纹理属于后端，控件只在第一次渲染时创建它。
        // 为什么不在构造时上传：构造时手里没有 `RenderCtx`（它只存在于渲染阶段）。
        let texture = match self.texture {
            Some(t) if t.is_valid() => t,
            _ => {
                let t = ctx.upload_texture(self.width, self.height, &self.rgba);
                self.texture = Some(t);
                t
            }
        };
        if !texture.is_valid() {
            // 后端不支持纹理（如还未实现 `upload_texture` 的旧版后端）。
            //
            // 这里**什么都不画**：既然能拿到有效句柄就一定会走 `draw_image`，
            // 不支持的后端会让它变成空白区域 —— 这是合理降级，不要画带误导性的框。
            return;
        }
        ctx.draw_image(self.rect, texture, self.style);
    }

    /// 告诉框架"我占用了哪张纹理"。
    ///
    /// 为什么不在 `Drop` 里释放：那时没有 `RenderCtx`，而纹理属于后端。
    /// 框架会把这些 id 堆到下一帧渲染前统一交给 `RenderCtx::release_texture`。
    fn release_textures(&mut self, out: &mut Vec<y901_ui::image::TextureId>) {
        if let Some(id) = self.texture.take() {
            if id.is_valid() {
                out.push(id);
            }
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("image {}", self.id))
            .prop("width", self.width.to_string())
            .prop("height", self.height.to_string())
            .prop("fit", format!("{:?}", self.style.fit))
    }
}
