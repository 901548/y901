//! Image 控件：持有一张 RGBA 位图，首次渲染时上传为纹理并绘制。

use y901_ui::image::{ImageFit, ImageStyle, TextureId};
use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, UiEvent,
    Widget,
};

/// 单边上限（像素）。
///
/// 为什么需要：`(w * h * 4)` 在 `usize` 上会溢出，而溢出后分配到的是
/// 一个"看起来合法"的小缓冲，后续越界读写就是 UB 前的未定义行为。
pub const MAX_IMAGE_SIDE: u32 = 16_384;
/// 总字节上限（128 MiB）。
pub const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;

/// 尺寸是否可接受（同时检查单边与乘法溢出）。
fn is_supported_size(w: u32, h: u32) -> bool {
    if w == 0 || h == 0 || w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return false;
    }
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .is_some_and(|bytes| bytes <= MAX_IMAGE_BYTES)
}

/// 图片控件。
pub struct Image {
    id: String,
    /// 无障碍名称（读屏器念它）。空 = 不提供名称（**不再拿 id 顶替**）。
    label: String,
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
    /// 是否已为收养的纹理做过一次 `retain`。
    ///
    /// 为什么记张标志：`retain` 需要 `RenderCtx`，而它只在渲染阶段存在；
    /// 重复 retain 会让计数偏高 → 纹理永不释放。
    retained: bool,
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
        // 超限的图片不能进入渲染路径：先在这里截断（不 panic、不分配），
        // 严格调用方请用 `try_from_rgba` 取得明确错误。
        if !is_supported_size(w, h) {
            eprintln!("[y901] Image::from_rgba 尺寸超限：{w}x{h}，已忽略");
            return Self::empty(id);
        }
        let need = (w as usize) * (h as usize) * 4;
        let mut data = Vec::with_capacity(need);
        data.extend_from_slice(&rgba[..rgba.len().min(need)]);
        data.resize(need, 0);
        Self {
            id: id.into(),
            label: String::new(),
            width: w,
            height: h,
            rgba: data,
            style: ImageStyle::default(),
            texture: None,
            retained: false,
            rect: Rect::ZERO,
            item: y901_ui::FlexItem::default(),
        }
    }

    /// 无障碍名称（读屏器念的名字；空 = 不提供）。
    ///
    /// 为什么不给默认值：语义里曾经写 `format!("… {}", self.id)`，于是读屏器把
    /// **内部 id** 念给用户听（真机 UIA 验收抓到的缺陷之一）。组件编不出人话，
    /// 就该留空让使用方补 —— 空名字比"念实现细节"好。
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// 从**已有纹理**创建（图标复用）。
    ///
    /// 输入：`id`、纹理句柄、原始尺寸（布局与 `ImageStyle::layout` 要用）。
    /// 行为：控件**不拥有**这张纹理，首次渲染时 `retain` 一次，卸载时 `release` 一次。
    /// 于是：多个 Image 可以共享同一张纹理，**最后一个卸载时才真正释放**。
    pub fn from_texture(
        id: impl Into<String>,
        texture: TextureId,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            width: width.max(1),
            height: height.max(1),
            // 不需要自己的位图；纹理已存在
            rgba: Vec::new(),
            style: ImageStyle::default(),
            texture: Some(texture),
            retained: false,
            rect: Rect::ZERO,
            item: y901_ui::FlexItem::default(),
        }
    }

    /// 严格版：尺寸/字节数超限返回 `Err`，而不是静默失败。
    ///
    /// 输入：`id`、宽高、`rgba`（必须 ≥ `w*h*4`）。
    /// 输出：`Err` 带可读原因（尺寸超限 / 字节数不足 / 乘法溢出）。
    ///
    /// 为什么要单独一个入口而不改 `from_rgba` 的签名：后者已被现有调用方使用，
    /// 改成 `Result` 会破坏所有构造点；而“不 panic”本来就是它的契约。
    pub fn try_from_rgba(
        id: impl Into<String>,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> Result<Self, String> {
        if !is_supported_size(w, h) {
            return Err(format!(
                "尺寸超限：{w}x{h}（上限 {MAX_IMAGE_SIDE}，总字节上限 {MAX_IMAGE_BYTES}）"
            ));
        }
        let need = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| format!("尺寸乘法溢出：{w}x{h}"))?;
        if rgba.len() < need {
            return Err(format!("字节数不足：需 {need}，实际 {}", rgba.len()));
        }
        Ok(Self::from_rgba(id, w, h, rgba))
    }

    /// 空图片（无位图、无纹理）——超限时的占位值。
    fn empty(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 0],
            style: ImageStyle::default(),
            texture: None,
            retained: false,
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
        // **`Cursor` 而不是裸 `&[u8]`**：`png` 0.18 的 `Decoder::new` 要求
        // `R: Read + Seek`，而 `&[u8]` 只实现 `Read`/`BufRead`（0.17 时代够用）。
        // `Cursor<&[u8]>` 不复制字节，只是加了个位置指针。
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("PNG 读取失败: {e}"))?;
        // **`None` 不 panic**：0.18 在尺寸溢出 `usize`（或头部信息不足）时返回 `None`，
        // 而调用方拿到的是 `Result`，就该报错而不是炸掉整个 UI。
        let need = reader
            .output_buffer_size()
            .ok_or_else(|| "PNG 解码缓冲尺寸无法确定（图像过大或头部损坏）".to_string())?;
        let mut buf = vec![0; need];
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
            Some(t) if t.is_valid() => {
                // 收养的纹理：第一次真正用到它时才 retain，而不是构造时（那时拿不到 ctx）。
                if !self.retained {
                    ctx.retain_texture(t);
                    self.retained = true;
                }
                t
            }
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
            .label(self.label.clone())
            .prop("width", self.width.to_string())
            .prop("height", self.height.to_string())
            .prop("fit", format!("{:?}", self.style.fit))
    }
}
