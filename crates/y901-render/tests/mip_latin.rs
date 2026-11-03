//! Latin 字形的 L1 mipmap 质量对比：**重光栅化 vs 盒式滤波**。
//!
//! 这一步不接入渲染路径，只回答一个问题：
//! 设计文档声称"每级重新光栅化优于从 L0 盒式降采样"，这句话到底成不成立？
//!
//! 跑法：
//! ```text
//! cargo test -p y901-render --release --test mip_latin -- --ignored --nocapture
//! ```

use y901_render::mip::MipLevel;
use y901_render::MyFontEngine;
use y901_ui::Size;

/// 主字号：与框架默认一致。
const BASE_PX: f32 = 15.0;
/// L1 的字号：一半。
const HALF_PX: f32 = BASE_PX / 2.0;

fn engine(px: f32, atlas: u32) -> MyFontEngine {
    let (cw, ch) = y901_render::cell_metrics_for(px);
    let cell = Size::new(cw, ch);
    MyFontEngine::with_atlas_size(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        px,
        cell,
        atlas,
    )
    .expect("engine")
}

/// 从图集里抠出一个字形的覆盖率位图（读 `r` 通道）。
///
/// 取 `&mut` 是因为 `FontEngine::glyph` 会现场光栅化并写图集（因此需要可变借用）。
fn extract(engine: &mut MyFontEngine, ch: char) -> Option<MipLevel> {
    // 先取几何信息，结束对 engine 的可变借用；再读图集像素。
    let (w, h, ax, ay) = {
        let g = engine.glyph(ch)?;
        (g.width, g.height, g.atlas_x as usize, g.atlas_y as usize)
    };
    let atlas_size = engine.atlas_size() as usize;
    let atlas = engine.atlas();
    let mut cov = vec![0u8; (w * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let si = ((ay + y) * atlas_size + (ax + x)) * 4;
            cov[y * w as usize + x] = atlas.get(si).copied().unwrap_or(0);
        }
    }
    Some(MipLevel::new(w, h, cov))
}

/// 两张尺寸可能不同的位图：裁到公共区域后比 MAD。
///
/// 为什么允许裁剪：重光栅化的位图尺寸一般等于 `round(w/2)`，与 `w/2` 可能差 1 像素；
/// 拒绝比较会让整组数据取不到。裁剪是这里唯一公平的做法。
fn mad_cropped(a: &MipLevel, b: &MipLevel) -> Option<f32> {
    let w = a.width.min(b.width);
    let h = a.height.min(b.height);
    if w == 0 || h == 0 {
        return None;
    }
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in 0..h {
        for x in 0..w {
            let ia = (y * a.width + x) as usize;
            let ib = (y * b.width + x) as usize;
            sum += (a.coverage[ia] as i16 - b.coverage[ib] as i16).unsigned_abs() as u64;
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    Some(sum as f32 / (255.0 * n as f32))
}

struct ClassStat {
    name: &'static str,
    n: usize,
    sum: f64,
    max: f32,
    min: f32,
    bytes_l1: usize,
    bytes_l0: usize,
}

#[test]
#[ignore = "质量对比基准：--ignored --nocapture 手动跑"]
fn latin_l1_rasterize_vs_box() {
    let mut base = engine(BASE_PX, 2048);
    let mut half = engine(HALF_PX, 1024);

    let classes: [(&str, Vec<char>); 3] = [
        ("大写字母", ('A'..='Z').collect()),
        ("小写字母", ('a'..='z').collect()),
        ("数字", ('0'..='9').collect()),
    ];

    let t0 = std::time::Instant::now();
    let mut total_chars = 0usize;
    let mut total_bytes_l1 = 0usize;
    let mut total_bytes_l0 = 0usize;
    let mut per_class: Vec<ClassStat> = Vec::new();

    for (name, chars) in &classes {
        let mut sum = 0.0f64;
        let mut max = 0.0f32;
        let mut min = 1.0f32;
        let mut n = 0usize;
        let mut bytes_l1 = 0usize;
        let mut bytes_l0 = 0usize;

        for &ch in chars {
            let l0 = match extract(&mut base, ch) {
                Some(l) => l,
                None => continue,
            };
            // 保证 L1 的字形已被光栅化（这里触发一次，不参与断言）
            let _ = half.glyph(ch);
            let l1_re = match extract(&mut half, ch) {
                Some(l) => l,
                None => continue,
            };
            let l1_box = l0.downsample_box();

            let mad = match mad_cropped(&l1_re, &l1_box) {
                Some(m) => m,
                None => continue,
            };
            sum += mad as f64;
            max = max.max(mad);
            min = min.min(mad);
            n += 1;
            bytes_l1 += l1_re.coverage.len();
            bytes_l0 += l0.coverage.len();
            total_chars += 1;
            total_bytes_l1 += l1_re.coverage.len();
            total_bytes_l0 += l0.coverage.len();
        }

        per_class.push(ClassStat {
            name,
            n,
            sum,
            max,
            min,
            bytes_l1,
            bytes_l0,
        });
    }
    let elapsed = t0.elapsed();

    eprintln!(
        "[latin-l1] {} 个字符（字号 {} → {}），总耗时 {:?}",
        total_chars, BASE_PX, HALF_PX, elapsed
    );
    eprintln!(
        "[latin-l1] | 字形类别 | 样本 | MAD 均值 | 最大 | 最小 | L1 字节 | L0 字节 |"
    );
    for c in &per_class {
        let mean = if c.n == 0 { 0.0 } else { c.sum / c.n as f64 };
        eprintln!(
            "[latin-l1] | {} | {} | {:.4} | {:.4} | {:.4} | {} | {} |",
            c.name, c.n, mean, c.max, c.min, c.bytes_l1, c.bytes_l0
        );
        assert!(c.n > 0, "{} 类应有样本", c.name);
    }
    eprintln!(
        "[latin-l1] L1 总字节 {} / L0 总字节 {} = {:.1}%（理论 25%）",
        total_bytes_l1,
        total_bytes_l0,
        100.0 * total_bytes_l1 as f64 / total_bytes_l0 as f64
    );

    // 变异对照：两条路径不能完全相同，否则这个测量没有区分力
    let mut base2 = engine(BASE_PX, 2048);
    let l0 = extract(&mut base2, 'm').expect("m");
    let l1_box = l0.downsample_box();
    let mut half2 = engine(HALF_PX, 1024);
    let _ = half2.glyph('m');
    let l1_re = extract(&mut half2, 'm').expect("m re");
    let mad_m = mad_cropped(&l1_re, &l1_box).expect("可比");
    eprintln!("[latin-l1] 变异对照 'm'：MAD = {mad_m:.4}");
    assert!(
        mad_m > 0.0,
        "重光栅化与盒式滤波结果完全相同 → 两条路径实际是同一条，测量没有区分力"
    );
}

#[test]
#[ignore = "质量对比基准：--ignored --nocapture 手动跑"]
fn latin_l1_which_method_is_closer_to_ideal() {
    // “两者不同”不等于“我们更好”。这里用一个**代理真值**回答“谁更接近理想”：
    // 在 4 倍字号下光栅化，再盒式降采样到 L1 尺寸 —— 高分辨率下的形状更接近理想轮廓。
    let mut base = engine(BASE_PX, 2048);
    let mut half = engine(HALF_PX, 1024);
    let mut quad = engine(BASE_PX * 4.0, 2048);

    let mut sum_re = 0.0f64;
    let mut sum_box = 0.0f64;
    let mut n = 0usize;
    let mut re_wins = 0usize;

    for ch in ('A'..='Z').chain('a'..='z').chain('0'..='9') {
        let l0 = match extract(&mut base, ch) { Some(l) => l, None => continue };
        let l1_re = match extract(&mut half, ch) { Some(l) => l, None => continue };
        let l1_box = l0.downsample_box();

        // 代理真值：4 倍字号 → 降采样 2 次
        let hi = match extract(&mut quad, ch) { Some(l) => l, None => continue };
        let gt = hi.downsample_box().downsample_box();

        let (a, b) = match (mad_cropped(&l1_re, &gt), mad_cropped(&l1_box, &gt)) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        sum_re += a as f64;
        sum_box += b as f64;
        if a <= b { re_wins += 1; }
        n += 1;
    }

    if n == 0 {
        eprintln!("[latin-gt] 没有可比较的样本");
        return;
    }
    let mean_re = sum_re / n as f64;
    let mean_box = sum_box / n as f64;
    eprintln!(
        "[latin-gt] 样本 {n}：重光栅化 MAD={mean_re:.4}，盒式滤波 MAD={mean_box:.4}，重光栅化更近的占 {}/{}",
        re_wins, n
    );
}

/// L2 的字号：四分之一。
const QUARTER_PX: f32 = BASE_PX / 4.0;

#[test]
#[ignore = "质量对比基准：--ignored --nocapture 手动跑"]
fn latin_l2_rasterize_vs_box() {
    let mut base = engine(BASE_PX, 2048);
    let mut quarter = engine(QUARTER_PX, 1024);

    let classes: [(&str, Vec<char>); 3] = [
        ("大写字母", ('A'..='Z').collect()),
        ("小写字母", ('a'..='z').collect()),
        ("数字", ('0'..='9').collect()),
    ];

    let t0 = std::time::Instant::now();
    let mut total = 0usize;
    let mut bytes_l2 = 0usize;
    let mut bytes_l0 = 0usize;

    eprintln!("[latin-l2] | 字形类别 | 样本 | MAD 均值 | 最大 | 最小 | L2 字节 |");
    for (name, chars) in &classes {
        let mut sum = 0.0f64;
        let mut max = 0.0f32;
        let mut min = 1.0f32;
        let mut n = 0usize;
        let mut b2 = 0usize;

        for &ch in chars {
            let l0 = match extract(&mut base, ch) { Some(l) => l, None => continue };
            let l2_re = match extract(&mut quarter, ch) { Some(l) => l, None => continue };
            // 盒式对照组：L0 → 降 1 次 → 降 2 次
            let l2_box = l0.downsample_box().downsample_box();

            let mad = match mad_cropped(&l2_re, &l2_box) { Some(m) => m, None => continue };
            sum += mad as f64;
            max = max.max(mad);
            min = min.min(mad);
            n += 1;
            b2 += l2_re.coverage.len();
            total += 1;
            bytes_l2 += l2_re.coverage.len();
            bytes_l0 += l0.coverage.len();
        }
        let mean = if n == 0 { 0.0 } else { sum / n as f64 };
        eprintln!(
            "[latin-l2] | {} | {} | {:.4} | {:.4} | {:.4} | {} |",
            name, n, mean, max, min, b2
        );
        assert!(n > 0, "{} 类应有样本", name);
    }
    let elapsed = t0.elapsed();
    eprintln!(
        "[latin-l2] {} 个字符（字号 {} → {}），耗时 {:?}，L2 总字节 {} / L0 {} = {:.1}%（理论 6.25%）",
        total, BASE_PX, QUARTER_PX, elapsed, bytes_l2, bytes_l0,
        100.0 * bytes_l2 as f64 / bytes_l0 as f64
    );

    // 变异对照：至少一个字形上两条路径不能完全相同
    let mut b = engine(BASE_PX, 2048);
    let mut q = engine(QUARTER_PX, 1024);
    let l0 = extract(&mut b, 'm').expect("m");
    let l2_box = l0.downsample_box().downsample_box();
    let _ = q.glyph('m');
    let l2_re = extract(&mut q, 'm').expect("m re");
    let mad_m = mad_cropped(&l2_re, &l2_box).expect("可比");
    eprintln!("[latin-l2] 变异对照 'm'：MAD = {mad_m:.4}");
    assert!(mad_m > 0.0, "L2 两条路径结果完全相同 → 测量没有区分力");
}
