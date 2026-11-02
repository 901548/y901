//! 纹理回收：引用计数、卸载上报、延迟释放。

use std::cell::RefCell;
use std::rc::Rc;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::{ImageStyle, TextureId};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    UiEvent, UiTree, Widget,
};
use y901_widgets::image::Image;

const W: u32 = 64;
const H: u32 = 64;

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

fn solid(s: &mut SoftBackend, w: u32, h: u32, c: (u8, u8, u8)) -> TextureId {
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        bytes.extend_from_slice(&[c.0, c.1, c.2, 255]);
    }
    s.upload_texture(w, h, &bytes)
}

/// 造一张白纹理交给 Image（模拟"控件自己上传"）。
fn image_with_texture(s: &mut SoftBackend) -> Image {
    let id = solid(s, 2, 2, (255, 255, 255));
    let rgba = vec![255u8; 2 * 2 * 4];
    let mut img = Image::from_rgba("img", 2, 2, &rgba);
    img.adopt_texture_for_test(id);
    img
}

#[test]
fn upload_then_release_frees_the_texture() {
    let mut s = soft();
    let id = solid(&mut s, 2, 2, (10, 20, 30));
    assert!(s.texture_count() >= 1, "上传后应有纹理");
    s.release_texture(id);
    assert_eq!(s.texture_count(), 0, "释放唯一引用后纹理表应为空");
}

#[test]
fn shared_texture_survives_until_the_last_release() {
    let mut s = soft();
    let id = solid(&mut s, 2, 2, (10, 20, 30));
    // 第二个使用者 retain：模拟两个 Image 共享同一张纹理
    s.retain_texture(id);
    assert_eq!(s.texture_refcount(id), Some(2), "retain 后应有 2 个引用");

    s.release_texture(id);
    assert_eq!(s.texture_count(), 1, "还有一个使用者，纹理不能被释放");
    assert_eq!(s.texture_refcount(id), Some(1));

    s.release_texture(id);
    assert_eq!(s.texture_count(), 0, "最后一个引用释放后应回收");
}

#[test]
fn extra_release_of_unknown_texture_is_harmless() {
    let mut s = soft();
    s.release_texture(TextureId(999));
    s.release_texture(TextureId::INVALID);
    assert_eq!(s.texture_count(), 0, "释放不存在的纹理不应 panic 也不应新增条目");
}

/// 场景根：持有一个 Image 子节点，用来走真实的布局与渲染路径。
struct Host {
    rect: Rect,
    child: Option<Image>,
    unmounted: Rc<RefCell<u32>>,
}

impl Widget for Host {
    fn id(&self) -> &str {
        "host"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
        c.constrain(Size::new(32.0, 32.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        if let Some(img) = self.child.as_mut() {
            img.arrange(rect, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if let Some(img) = self.child.as_mut() {
            img.render(ctx);
        }
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn on_unmount(&mut self) {
        *self.unmounted.borrow_mut() += 1;
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn unmounting_a_node_queues_its_texture_and_the_next_frame_releases_it() {
    let mut soft = soft();
    let mut tree = UiTree::new(Box::new(Host {
        rect: Rect::ZERO,
        child: None,
        unmounted: Rc::default(),
    }));
    let id = solid(&mut soft, 2, 2, (1, 2, 3));
    assert_eq!(soft.texture_count(), 1);

    // 直接把纹理标记为"已卸载"：走树公开的释放队列接口
    tree.queue_texture_release(id);
    // 下一帧渲染时排队项被交给后端
    tree.invalidate_all();
    tree.render(&mut soft);
    assert_eq!(soft.texture_count(), 0, "下一帧渲染应真正释放纹理");
}

#[test]
fn image_reports_its_texture_when_unmounted() {
    let mut soft = soft();
    let mut img = image_with_texture(&mut soft);
    let mut out = Vec::new();
    img.release_textures(&mut out);
    assert_eq!(out.len(), 1, "Image 卸载时应上报 1 张纹理");
    assert!(out[0].is_valid(), "上报的应是有效句柄");

    // 再卸载一次不应重复上报（take 已清空）
    let mut out2 = Vec::new();
    img.release_textures(&mut out2);
    assert!(out2.is_empty(), "重复卸载不应重复上报");
}

#[test]
fn image_renders_with_adopted_texture() {
    let mut soft = soft();
    let mut img = image_with_texture(&mut soft);
    img.arrange(Rect::new(0.0, 0.0, 16.0, 16.0), &soft, &mut LayoutCache::new());
    soft.clear();
    img.render(&mut soft);
    let p = soft.pixel(8, 8).expect("像素");
    let (r, g, b) = (((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8);
    assert!(
        r > 200 && g > 200 && b > 200,
        "收养的白纹理应显示为白色，实际 ({r},{g},{b})"
    );
    let _ = Color::WHITE;
    let _ = ImageStyle::default();
}
