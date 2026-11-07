//! 四个边界风险的回归测试（v0.1.1）。
//!
//! 每个用例都对应一个"真实会发生的输入"，不是人造的极端值：
//! 拖拽算出的 NaN、用户粘贴 10 MB、超大图片、零尺寸网格。

use y901_ui::grid::{Grid, GridTrack};
use y901_ui::text::{TextBuffer, MAX_TEXT_LEN};
use y901_widgets::image::{Image, MAX_IMAGE_SIDE};
use y901_widgets::Slider;

#[test]
fn slider_rejects_nan_and_infinity() {
    // NaN 会一路流进布局与提交，最后变成整帧不可见的黑屏。
    assert_eq!(Slider::new("a", f32::NAN).value(), 0.0);
    assert_eq!(Slider::new("b", f32::INFINITY).value(), 1.0);
    assert_eq!(Slider::new("c", f32::NEG_INFINITY).value(), 0.0);

    let mut s = Slider::new("d", 0.5);
    s.set_value(f32::NAN);
    assert_eq!(s.value(), 0.0, "set_value(NaN) 应归零，而不是保留 NaN");
    s.set_value(f32::INFINITY);
    assert_eq!(s.value(), 1.0, "+∞ 应夹到 1");
    s.set_value(-2.0);
    assert_eq!(s.value(), 0.0);
}

#[test]
fn text_buffer_truncates_at_the_limit_instead_of_panicking() {
    let mut b = TextBuffer::new();
    // 10 MiB：真实粘贴规模，不是人造极端值
    let big = "a".repeat(10 * 1024 * 1024);
    b.insert_str(&big);
    assert_eq!(b.len_bytes(), 10 * 1024 * 1024, "未超限时不应截断");

    // 再塞 10 MiB：应截断到上限，且光标停在边界上
    b.insert_str(&big);
    assert_eq!(b.len_bytes(), MAX_TEXT_LEN, "总长度应被截断到 MAX_TEXT_LEN");
    assert!(
        b.text().is_char_boundary(b.cursor()),
        "截断后光标必须仍在 char 边界（否则后续 insert 会 panic）"
    );
}

#[test]
fn text_buffer_truncation_never_splits_a_multibyte_char() {
    let mut b = TextBuffer::new();
    // 先用 3 字节字符把长度推到接近上限，再塞一个会跨越边界的串
    let chunk = "中".repeat(MAX_TEXT_LEN / 3);
    b.insert_str(&chunk);
    let before = b.len_bytes();
    b.insert_str("abc");
    assert_eq!(b.len_bytes(), MAX_TEXT_LEN, "应补齐到上限");
    assert!(
        b.text().is_char_boundary(b.cursor()),
        "cursor={} 不在字符边界上",
        b.cursor()
    );
    assert!(before <= MAX_TEXT_LEN);
}

#[test]
fn image_rejects_oversized_dimensions() {
    // 单边超限
    let err = Image::try_from_rgba("big", MAX_IMAGE_SIDE + 1, 1, &[0; 4]);
    assert!(err.is_err(), "单边超限应返回 Err");

    // 乘法溢出：16384 × 16384 × 4 = 1 GiB > 128 MiB 上限
    let err = Image::try_from_rgba("wide", 16_384, 16_384, &[0; 4]);
    assert!(err.is_err(), "总字节超限应返回 Err");

    // 正常尺寸可以
    let ok = Image::try_from_rgba("ok", 2, 2, &[255; 16]);
    assert!(ok.is_ok(), "正常尺寸应成功");

    // 字节数不足
    let err = Image::try_from_rgba("short", 2, 2, &[0; 3]);
    assert!(err.is_err(), "字节数不足应返回 Err");
}

#[test]
fn image_from_rgba_on_oversize_degrades_instead_of_panicking() {
    // 宽容入口（不改签名）遇到超限时给一个 1×1 空图，而不是分配 1 GiB
    let img = Image::from_rgba("huge", 100_000, 100_000, &[0; 4]);
    assert_eq!(img.image_size(), (1, 1), "超限时应退化为 1×1 占位");
}

#[test]
fn grid_accepts_zero_and_non_finite_tracks_without_panicking() {
    // 零尺寸/非有限轨道：入口收敛为 0，内部不会出现 NaN 矩形
    let g = Grid::new(
        "g",
        vec![
            GridTrack::Fixed(0.0),
            GridTrack::Fixed(f32::NAN),
            GridTrack::Fraction(f32::INFINITY),
            GridTrack::Fixed(-5.0),
        ],
        vec![GridTrack::Fixed(f32::NAN)],
    );
    // 只要求"能构造、不 panic"；具体分配由布局阶段决定
    let _ = g;
}
