//! 自研 TTF 字体引擎 —— Milestone 1：轮廓展平 + 扫描线光栅化。
//!
//! 本模块只解决"把一个字形变成覆盖度位图"，**不接图集、不做亚像素、不做 hinting**。
//!
//! 三个硬约束（来自《字体引擎战略地图》）：
//! 1. 坐标全程 **26.6 定点**（一个单位 = 1/64 px）；
//! 2. 曲线自适应展平，平坦度阈值 **0.25px**；直线段直接保留；
//! 3. 扫描线用**非零环绕规则**，半开区间 `[ymin, ymax)` 避免顶点双计。

mod engine;
mod flatten;
mod raster;

pub use engine::MyFontEngine;
pub use flatten::{flatten_glyph, Line};
pub use raster::{rasterize, Coverage};

/// 一个光栅化好的字形。
#[derive(Clone, Debug)]
pub struct GlyphBitmap {
    /// 位图宽度（像素）。
    pub width: u32,
    /// 位图高度（像素）。
    pub height: u32,
    /// 墨迹相对 pen 原点的 x 偏移（像素）。
    pub bearing_x: i32,
    /// 墨迹相对基线的 y 偏移（像素，向下为正）。
    pub bearing_y: i32,
    /// 前进宽度（像素）。
    pub advance: f32,
    /// 覆盖度（每像素一字节，0..=255），行主序。
    pub coverage: Vec<u8>,
}

impl GlyphBitmap {
    /// 覆盖度总和 / 255 = 墨量（即"多少个全黑像素"）。
    pub fn ink_area(&self) -> f64 {
        self.coverage.iter().map(|c| *c as f64 / 255.0).sum()
    }

    /// 位图中某像素的覆盖度（越界返回 0）。
    pub fn at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.coverage[(y * self.width + x) as usize]
    }

    /// 非空像素的连通域个数（四连通，阈值 `thr`）。
    ///
    /// 用途：`o`/`e` 必须等于 2（外轮廓 + 内孔之间的"环"与"内部空白"）。
    pub fn components(&self, thr: u8) -> usize {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut seen = vec![false; w * h];
        let mut count = 0usize;
        let mut stack = Vec::new();
        for start in 0..w * h {
            if seen[start] || self.coverage[start] <= thr {
                continue;
            }
            count += 1;
            stack.push(start);
            seen[start] = true;
            while let Some(i) = stack.pop() {
                let (x, y) = (i % w, i / w);
                let push = |nx: usize, ny: usize, stack: &mut Vec<usize>, seen: &mut Vec<bool>| {
                    let j = ny * w + nx;
                    if !seen[j] && self.coverage[j] > thr {
                        seen[j] = true;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    push(x - 1, y, &mut stack, &mut seen);
                }
                if x + 1 < w {
                    push(x + 1, y, &mut stack, &mut seen);
                }
                if y > 0 {
                    push(x, y - 1, &mut stack, &mut seen);
                }
                if y + 1 < h {
                    push(x, y + 1, &mut stack, &mut seen);
                }
            }
        }
        count
    }
}

/// 从字体字节中取一个字形并光栅化。
///
/// 输入：`font_data`（TTF/OTF 字节，必须常驻）、`ch`、`size_px`（逻辑像素字号）。
/// 返回：`None` = 字体解析失败或该字符无轮廓（空格等）。
/// 副作用：无（不写任何缓存，M1 阶段故意）。
///
/// **为什么 `Face` 每次临时构造**：`ttf_parser::Face<'a>` 借用字体字节，
/// 若把它存进结构体就得弄自引用（`ouroboros` 那类）。构造成本只是几个表偏移的计算，
/// 相对光栅化（十微秒级）可忽略。
pub fn rasterize_glyph(font_data: &[u8], ch: char, size_px: f32) -> Option<GlyphBitmap> {
    rasterize_glyph_phased(font_data, ch, size_px, 0)
}

/// 带**亚像素相位**的光栅化：轮廓在 x 轴平移 `phase/4` 个像素。
///
/// 输入：`phase` ∈ 0..=3（对应 0 / 0.25 / 0.5 / 0.75 px）。
/// 为什么是"真平移"而非"取整"：取整会让一行里每个字的左右间距跳 0.5px，
/// **单个字形的最大边长**（像素）。
///
/// 4096 是常见 GPU 纹理上限，也足够覆盖任何真实字体（字号 < 3000px）。
/// 超过它基本只能是字体文件损坏。
pub const MAX_GLYPH_DIM: u32 = 4096;


/// **字形的推进宽度**（像素）——直接用字体的 `hmtx`。
///
/// 为何需要它：框架的格子布局把**所有 ASCII** 算作 1 格（固定 `cell_w`），
/// 对等宽字体正确；但**比例字体**（微软雅黑、宋体等）里 i 的推进宽度只有 m 的一半，
/// 按固定格宽画就会密密匐匐地挤在一起（看起来像字在跳舞）。
///
/// 返回：`None` = 字体没有该字形（调用方应回退到格宽）。
pub fn glyph_advance_px(font_data: &[u8], ch: char, size_px: f32) -> Option<f32> {
    let face = ttf_parser::Face::parse(font_data, 0).ok()?;
    let gid = face.glyph_index(ch)?;
    let units = face.units_per_em() as f32;
    if units <= 0.0 || size_px <= 0.0 || !size_px.is_finite() {
        return None;
    }
    let adv = face.glyph_hor_advance(gid)? as f32;
    Some(adv * size_px / units)
}

/// **光栅化一个字形**（带亚像素相位），返回覆盖度位图。
///
/// 返回：`None` = 字体里没有该字形（且替身字符也没有），或尺寸超安全上限。
pub fn rasterize_glyph_phased(
    font_data: &[u8],
    ch: char,
    size_px: f32,
    phase: u8,
) -> Option<GlyphBitmap> {
    let face = ttf_parser::Face::parse(font_data, 0).ok()?;
    // 缺字时先试替身字符（当前只有 U+2212 → '-' 一条）。
    let gid = match face.glyph_index(ch) {
        Some(g) => g,
        None => match crate::font_fallback::substitute(ch) {
            Some(sub) => face.glyph_index(sub)?,
            None => return None,
        },
    };
    let units = face.units_per_em() as f32;
    if units <= 0.0 || size_px <= 0.0 {
        return None;
    }
    let scale = size_px / units;

    // 26.6：先用 f32 算到像素，再转 i32 定点（四舍五入，避免截断偏差）
    let (lines, advance) = flatten::flatten_glyph(&face, gid, scale)?;
    if lines.is_empty() {
        return None;
    }

    // 包围盒（含 1px 留边，保证抗锯不被裁）
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for l in &lines {
        for (x, y) in [(l.x0, l.y0), (l.x1, l.y1)] {
            let x = x as f32 / 64.0;
            let y = y as f32 / 64.0;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    let bearing_x = min_x.floor() as i32;
    let bearing_y = min_y.floor() as i32;
    let width = (max_x.ceil() as i32 - bearing_x).max(1) as u32 + 1;
    let height = (max_y.ceil() as i32 - bearing_y).max(1) as u32 + 1;

    // **字形尺寸安全限制**：包围盒来自**字体文件里的坐标**，
    // 损坏/恶意的 TTF 可以把顶点写成巨大值，使导出图像达到数万像素。
    // 不拦就会在 `rasterize` 里 `vec![0f32; w*h]` 直接 OOM。
    // 此处拦在**分配之前**，并明确告知（而不是静默失败）。
    if width > MAX_GLYPH_DIM || height > MAX_GLYPH_DIM || !raster::area_is_safe(width, height) {
        eprintln!(
            "[y901] 字形尺寸异常，已拒绝光栅化: {}x{} (字体坐标可能损坏)",
            width, height
        );
        return None;
    }

    // 亚像素平移：phase/4 像素 = phase*16，单位仍是 26.6（1/64 px）
    let subpixel = (phase as i32 & 3) * 16;
    // 平移到位图坐标（仍然 26.6）
    let ox = (bearing_x as i64) * 64;
    let oy = (bearing_y as i64) * 64;
    let shifted: Vec<Line> = lines
        .iter()
        .map(|l| Line {
            // 亚像素平移是**向右**的：phase 越大，字形越往右
            x0: l.x0 - ox as i32 + subpixel,
            y0: l.y0 - oy as i32,
            x1: l.x1 - ox as i32 + subpixel,
            y1: l.y1 - oy as i32,
        })
        .collect();

    let cov = raster::rasterize(&shifted, width, height)?;
    Some(GlyphBitmap {
        width,
        height,
        bearing_x,
        bearing_y,
        advance: advance * scale,
        coverage: cov.0,
    })
}

/// 对扁平化后的多边形计算**带符号面积**（shoelace，单位：像素²）。
///
/// 用途：面积恒等式——`覆盖度总和/255` 必须等于 `|多边形面积|`。
/// 外圈顺时针、内孔逆时针 → 代数和 = 实际墨量（内孔被自然扣除）。
pub fn polygon_area(lines: &[Line]) -> f64 {
    let mut sum = 0.0f64;
    for l in lines {
        let x0 = l.x0 as f64 / 64.0;
        let y0 = l.y0 as f64 / 64.0;
        let x1 = l.x1 as f64 / 64.0;
        let y1 = l.y1 as f64 / 64.0;
        sum += x0 * y1 - x1 * y0;
    }
    (sum * 0.5).abs()
}

/// 一个字形的几何信息（不光栅化，供对比用）。
pub struct GlyphGeometry {
    /// 展平后的线段（已按字号缩放到 26.6 像素）。
    pub lines: Vec<Line>,
    /// 前进宽度（像素）。
    pub advance: f32,
}

/// 只取几何（不光栅化），供"面积恒等式"与对比测试使用。
pub fn glyph_geometry(font_data: &[u8], ch: char, size_px: f32) -> Option<GlyphGeometry> {
    let face = ttf_parser::Face::parse(font_data, 0).ok()?;
    let gid = face.glyph_index(ch)?;
    let units = face.units_per_em() as f32;
    let scale = size_px / units;
    let (lines, advance) = flatten::flatten_glyph(&face, gid, scale)?;
    Some(GlyphGeometry {
        lines,
        advance: advance * scale,
    })
}

/// 便利：读一个系统字体到内存（常驻）。
pub fn load_font(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("读字体失败 {path}: {e}"))
}

/// 根据族名找字体文件（只支持几个常见 Windows 字体，M2 不做完整回退链）。
pub fn resolve_font_path(family: &str) -> String {
    let f = family.to_ascii_lowercase();
    let candidates: &[&str] = if f.contains("consolas") {
        &[
            "C:\\Windows\\Fonts\\consola.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ]
    } else if f.contains("cascadia") {
        &[
            "C:\\Windows\\Fonts\\CascadiaMono.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ]
    } else if f.contains("nsimsun") || f.contains("simsun") {
        &[
            "C:\\Windows\\Fonts\\simsun.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        ]
    } else {
        &[
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
    };
    for c in candidates {
        if std::path::Path::new(c).exists() {
            return (*c).to_string();
        }
    }
    default_test_font_path()
}

/// 默认字体（**按平台选择实际存在的一个**）。
///
/// 为什么不写死一个路径：这个函数之前返回 `C:\Windows\Fonts\simsun.ttc`，
/// 一到 Linux 就报 `No such file or directory`——这是"平台无关层"里唯一该分平台的地方（资源路径）。
pub fn default_test_font_path() -> String {
    const CANDIDATES: &[&str] = &[
        // Windows（与旧行为一致：一直用 Segoe UI）
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
        // Linux（Debian/Ubuntu 常见）
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    ];
    for c in CANDIDATES {
        if std::path::Path::new(c).exists() {
            return (*c).to_string();
        }
    }
    // 都不存在时返回第一个：让调用方的错误信息里带上路径，而不是报"找不到字体"
    CANDIDATES[0].to_string()
}

/// 渲染一个字符到终端（调试用：`#` = 不透明度高，`.` = 低）。
pub fn debug_ascii(g: &GlyphBitmap) -> String {
    let mut s = String::new();
    for y in 0..g.height {
        for x in 0..g.width {
            let c = g.at(x, y);
            s.push(match c {
                0..=8 => ' ',
                9..=80 => '.',
                81..=180 => '+',
                _ => '#',
            });
        }
        s.push('\n');
    }
    s
}
