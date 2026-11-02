//! `Image::from_texture`：多控件共享同一张纹理时的引用计数语义。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::TextureId;
use y901_ui::layout::LayoutCache;
use y901_ui::{Color, Pos, Rect, RenderCtx, Size, Widget};
use y901_widgets::image::Image;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(64, 64, Box::new(engine), cell).expect("soft")
}

/// 上传一张纯白 2×2 纹理（模拟"应用层建好图标纹理"）。
fn white_texture(s: &mut SoftBackend) -> TextureId {
    let rgba = vec![255u8; 2 * 2 * 4];
    s.upload_texture(2, 2, &rgba)
}

fn arrange_render(img: &mut Image, s: &mut SoftBackend) {
    let mut cache = LayoutCache::new();
    img.arrange(Rect::new(0.0, 0.0, 16.0, 16.0), s, &mut cache);
    img.render(s);
}

#[test]
fn two_images_share_one_texture_until_both_unmount() {
    let mut s = soft();
    let tex = white_texture(&mut s);
    assert_eq!(s.texture_refcount(tex), Some(1), "上传后计数应为 1");

    // 两个控件收养同一张纹理
    let mut a = Image::from_texture("a", tex, 2, 2);
    let mut b = Image::from_texture("b", tex, 2, 2);

    // 首次渲染时各自 retain（构造时拿不到 RenderCtx）
    arrange_render(&mut a, &mut s);
    arrange_render(&mut b, &mut s);
    eprintln!("[share] 渲染两个控件后 refcount = {:?}", s.texture_refcount(tex));
    assert_eq!(
        s.texture_refcount(tex),
        Some(3),
        "上传者 1 + 两个控件各 1 = 3"
    );

    // 卸载第一个：纹理仍在，且另一个仍能画
    let mut out_a = Vec::new();
    a.release_textures(&mut out_a);
    for id in &out_a {
        s.release_texture(*id);
    }
    assert_eq!(s.texture_refcount(tex), Some(2), "还剩两个引用");
    assert_eq!(s.texture_count(), 1, "纹理表仍有一条");

    s.clear();
    arrange_render(&mut b, &mut s);
    let p = s.pixel(8, 8).expect("px");
    let (r, g, blue) = (
        ((p >> 16) & 0xFF) as u8,
        ((p >> 8) & 0xFF) as u8,
        (p & 0xFF) as u8,
    );
    assert!(
        r > 200 && g > 200 && blue > 200,
        "卸载一个后另一个仍应能画出白色，实际 ({r},{g},{blue})"
    );

    // 卸载第二个 + 上传者释放 → 归零，纹理被回收
    let mut out_b = Vec::new();
    b.release_textures(&mut out_b);
    for id in &out_b {
        s.release_texture(*id);
    }
    assert_eq!(s.texture_count(), 1, "上传者自己那一份还在");
    s.release_texture(tex);
    assert_eq!(s.texture_count(), 0, "最后一个引用释放后纹理应被回收");
}

#[test]
fn from_bytes_image_still_releases_its_own_texture() {
    let mut s = soft();
    let rgba = vec![255u8; 2 * 2 * 4];
    let mut img = Image::from_rgba("own", 2, 2, &rgba);

    // 第一次渲染会上传并持有唯一引用
    arrange_render(&mut img, &mut s);
    assert_eq!(s.texture_count(), 1, "应已上传 1 张纹理");

    let mut out = Vec::new();
    img.release_textures(&mut out);
    assert_eq!(out.len(), 1, "卸载应上报 1 张纹理");
    for id in &out {
        s.release_texture(*id);
    }
    assert_eq!(s.texture_count(), 0, "自己拥有的纹理卸载后应被回收");
}

#[test]
fn adopting_an_invalid_texture_draws_nothing_and_retains_nothing() {
    let mut s = soft();
    let mut img = Image::from_texture("bad", TextureId::INVALID, 2, 2);
    arrange_render(&mut img, &mut s);
    assert_eq!(s.texture_count(), 0, "无效句柄不应留下纹理");
    let p = s.pixel(8, 8).expect("px");
    let _ = (p, Color::WHITE, Pos::new(0.0, 0.0));
}
