//! 轮廓展平：TTF/OTF 字形 → 直线段（26.6 定点）。

/// 一条线段（直线），坐标为 26.6 定点（1 单位 = 1/64 像素）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    /// 起点 x（26.6）。
    pub x0: i32,
    /// 起点 y（26.6）。
    pub y0: i32,
    /// 终点 x（26.6）。
    pub x1: i32,
    /// 终点 y（26.6）。
    pub y1: i32,
}

impl Line {
    #[inline]
    fn is_horizontal(&self) -> bool {
        self.y0 == self.y1
    }
    #[inline]
    fn is_degenerate(&self) -> bool {
        self.x0 == self.x1 && self.y0 == self.y1
    }
}

/// 平坦度阈值：0.25 px → 0.25*64 = 16（26.6）。
const TOL_26_6: i32 = 16;

/// 单个字形允许的**最多折线数**。
///
/// 取值依据：常见 CJK 字形经展平后约 50–300 条（笔画转折 + 曲线分解），
/// 复杂的中心字（如「龙」「龟」）也在千条以内。
/// 留 **10000** 是一个数量级的余量：真实字体碰不到，而恶意轮廓会先撞墅。
const MAX_LINES: usize = 10_000;

/// 单个字形允许的**最多轮廓数**（`move_to` 次数）。
///
/// 取值依据：汉字最多几十个笔画分支（含孔洞），字母更少。
/// **256** 已是实际上限的数倍。
const MAX_CONTOURS: usize = 256;

/// 收集轮廓的回调（`ttf_parser::OutlineBuilder`）。
pub struct Flattener {
    lines: Vec<Line>,
    /// 当前点，26.6。
    cur: Option<(i32, i32)>,
    /// 轮廓起点（26.6），`close` 时用。
    start: (i32, i32),
    /// font unit → 26.6 的缩放系数。
    scale: f32,
    /// 已开始的轮廓数（`move_to` 计数）。
    contours: usize,
    /// 是否已超限。超限后**继续消费回调但不再记录**，
    /// 因为 `ttf-parser` 的 `OutlineBuilder` 没有提前终止机制；
    /// 我们只能让自己的展平器拒收，不能让它解析出错。
    overflowed: bool,
}

impl Flattener {
    fn new(scale: f32) -> Self {
        Self {
            lines: Vec::new(),
            cur: None,
            start: (0, 0),
            scale,
            contours: 0,
            overflowed: false,
        }
    }

    #[inline]
    fn conv(&self, x: f32, y: f32) -> (i32, i32) {
        // px = v * scale；26.6 就是 px * 64。四舍五入避免截断偏差。
        let px = x * self.scale * 64.0;
        let py = y * self.scale * 64.0;
        (px.round() as i32, py.round() as i32)
    }

    fn push_line(&mut self, a: (i32, i32), b: (i32, i32)) {
        // 超限后不再追加任何折线：既保证内存不涨，也保证
        // 后续的扫描线不会读到一个截断的轮廓（它会因 `overflowed` 而整体被拒收）。
        if self.overflowed || self.lines.len() >= MAX_LINES {
            self.overflowed = true;
            return;
        }
        let l = Line {
            x0: a.0,
            y0: a.1,
            x1: b.0,
            y1: b.1,
        };
        if l.is_degenerate() {
            return;
        }
        // **水平线必须保留**！
        // 它们对扫描线交点确实零贡献（rasterize 里会跳过 dy==0），
        // 但它们是**轮廓的一部分**：删掉会把多边形打断成开链，
        // 导致环绕数不能成对、面积也算错（实测：'A' 墨量比真值大 30%）。
        self.lines.push(l);
    }

    /// 二次贝塞尔自适应展平（de Casteljau 中点分割）。
    fn quad(&mut self, p0: (i32, i32), p1: (i32, i32), p2: (i32, i32), depth: u32) {
        if depth >= 10 {
            self.push_line(p0, p2);
            return;
        }
        if Self::flat_enough(p0, p1, p2) {
            self.push_line(p0, p2);
            return;
        }
        let mid = |a: (i32, i32), b: (i32, i32)| -> (i32, i32) {
            (
                ((a.0 as i64 + b.0 as i64) / 2) as i32,
                ((a.1 as i64 + b.1 as i64) / 2) as i32,
            )
        };
        let p01 = mid(p0, p1);
        let p12 = mid(p1, p2);
        let m = mid(p01, p12);
        self.quad(p0, p01, m, depth + 1);
        self.quad(m, p12, p2, depth + 1);
    }

    /// 三次贝塞尔自适应展平。
    fn cubic(
        &mut self,
        p0: (i32, i32),
        p1: (i32, i32),
        p2: (i32, i32),
        p3: (i32, i32),
        depth: u32,
    ) {
        if depth >= 12 {
            self.push_line(p0, p3);
            return;
        }
        if Self::cubic_flat_enough(p0, p1, p2, p3) {
            self.push_line(p0, p3);
            return;
        }
        let mid = |a: (i32, i32), b: (i32, i32)| -> (i32, i32) {
            (
                ((a.0 as i64 + b.0 as i64) / 2) as i32,
                ((a.1 as i64 + b.1 as i64) / 2) as i32,
            )
        };
        let p01 = mid(p0, p1);
        let p12 = mid(p1, p2);
        let p23 = mid(p2, p3);
        let p012 = mid(p01, p12);
        let p123 = mid(p12, p23);
        let m = mid(p012, p123);
        self.cubic(p0, p01, p012, m, depth + 1);
        self.cubic(m, p123, p23, p3, depth + 1);
    }

    /// 二次平坦度：控制点到弦的垂距 ≤ TOL。
    ///
    /// 为什么用"垂距"而不是"两点距离"：两点距离在弦很长时会误判
    /// （控制点远但曲线近乎直线）。垂距才是真正的字形偏差。
    fn flat_enough(p0: (i32, i32), p1: (i32, i32), p2: (i32, i32)) -> bool {
        let (dx, dy) = (p2.0 - p0.0, p2.1 - p0.1);
        let len2 = dx as i64 * dx as i64 + dy as i64 * dy as i64;
        if len2 == 0 {
            // 弦退化：用控制点到起点的距离
            let (cx, cy) = (p1.0 - p0.0, p1.1 - p0.1);
            return (cx as i64 * cx as i64 + cy as i64 * cy as i64) <= (TOL_26_6 as i64).pow(2);
        }
        // 叉积 / 弦长 = 垂距；两边平方避开 sqrt：
        // cross² <= TOL² * len²
        let cross = dx as i64 * (p1.1 - p0.1) as i64 - dy as i64 * (p1.0 - p0.0) as i64;
        cross * cross <= (TOL_26_6 as i64).pow(2) * len2
    }

    /// 三次平坦度：两个控制点都足够近弦。
    fn cubic_flat_enough(p0: (i32, i32), p1: (i32, i32), p2: (i32, i32), p3: (i32, i32)) -> bool {
        let (dx, dy) = (p3.0 - p0.0, p3.1 - p0.1);
        let len2 = dx as i64 * dx as i64 + dy as i64 * dy as i64;
        if len2 == 0 {
            let d1 = (p1.0 - p0.0) as i64 * (p1.0 - p0.0) as i64
                + (p1.1 - p0.1) as i64 * (p1.1 - p0.1) as i64;
            let d2 = (p2.0 - p0.0) as i64 * (p2.0 - p0.0) as i64
                + (p2.1 - p0.1) as i64 * (p2.1 - p0.1) as i64;
            let t = (TOL_26_6 as i64).pow(2);
            return d1.max(d2) <= t;
        }
        let c1 = dx as i64 * (p1.1 - p0.1) as i64 - dy as i64 * (p1.0 - p0.0) as i64;
        let c2 = dx as i64 * (p2.1 - p0.1) as i64 - dy as i64 * (p2.0 - p0.0) as i64;
        let t = (TOL_26_6 as i64).pow(2) * len2;
        c1 * c1 <= t && c2 * c2 <= t
    }
}

impl ttf_parser::OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours += 1;
        if self.contours > MAX_CONTOURS {
            self.overflowed = true;
        }
        let p = self.conv(x, y);
        self.cur = Some(p);
        self.start = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.conv(x, y);
        if let Some(c) = self.cur {
            self.push_line(c, p);
        }
        self.cur = Some(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p1 = self.conv(x1, y1);
        let p2 = self.conv(x, y);
        if let Some(p0) = self.cur {
            self.quad(p0, p1, p2, 0);
        }
        self.cur = Some(p2);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p1 = self.conv(x1, y1);
        let p2 = self.conv(x2, y2);
        let p3 = self.conv(x, y);
        if let Some(p0) = self.cur {
            self.cubic(p0, p1, p2, p3, 0);
        }
        self.cur = Some(p3);
    }

    fn close(&mut self) {
        if let Some(c) = self.cur {
            if c != self.start {
                self.push_line(c, self.start);
            }
        }
        self.cur = Some(self.start);
    }
}

/// 把一个字形展平成直线段。
///
/// 返回：`(lines, advance_font_units)`；字形不存在或无轮廓时返回 `None`。
/// 副作用：无。
pub fn flatten_glyph(
    face: &ttf_parser::Face<'_>,
    gid: ttf_parser::GlyphId,
    scale: f32,
) -> Option<(Vec<Line>, f32)> {
    let mut f = Flattener::new(scale);
    face.outline_glyph(gid, &mut f)?;
    if f.overflowed {
        // 与“字形不存在”同一种处理：上层返回 `None`，不 panic、不崩。
        // 诊断只打一次（按字形去重），否则每帧都会刷屏。
        warn_overflow_once(gid.0);
        return None;
    }
    Some((f.lines, face.glyph_hor_advance(gid)? as f32))
}

/// 未使用的占位（保留 `Line` 的辅助方法以备扩展）。
// 测试 API：`_unused`：类型见证，保证 `Line` 仍被引用
#[allow(dead_code)]
fn _unused(l: Line) -> bool {
    l.is_horizontal() || l.is_degenerate()
}

/// 超限诊断的去重集：同一个字形只打一条日志。
///
/// 为什么要去重：超限字形如果在突发渲染中被反复请求，不去重会每帧写一条日志。
fn warn_overflow_once(gid: u16) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static SEEN: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let first = match seen.lock() {
        Ok(mut g) => g.insert(gid),
        Err(p) => p.into_inner().insert(gid),
    };
    if first {
        eprintln!(
            "[flatten] 字形 gid={gid} 超出上限（折线 ≤ {MAX_LINES}，轮廓 ≤ {MAX_CONTOURS}），已拒收该字形"
        );
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use ttf_parser::OutlineBuilder;

    fn flat() -> Flattener {
        Flattener::new(1.0)
    }

    /// 恰好 `MAX_LINES` 条折线：不应超限（防 off-by-one）。
    #[test]
    fn exactly_max_lines_is_accepted() {
        let mut f = flat();
        f.move_to(0.0, 0.0);
        // 每次 line_to 产生 1 条折线；第一次需要一个起点，所以多画 1 次
        for i in 0..MAX_LINES {
            f.line_to(i as f32, 1.0);
        }
        assert!(!f.overflowed, "恰好 {MAX_LINES} 条不应超限");
        assert!(f.lines.len() <= MAX_LINES);
    }

    /// `MAX_LINES + 1` 条：必须超限。
    #[test]
    fn max_lines_plus_one_overflows() {
        let mut f = flat();
        f.move_to(0.0, 0.0);
        for i in 0..=MAX_LINES {
            f.line_to(i as f32, 1.0);
        }
        assert!(f.overflowed, "超过 {MAX_LINES} 条应被拒收");
        assert_eq!(f.lines.len(), MAX_LINES, "超限后不应再追加折线");
    }

    /// 恰好 `MAX_CONTOURS` 个轮廓：不应超限。
    #[test]
    fn exactly_max_contours_is_accepted() {
        let mut f = flat();
        for i in 0..MAX_CONTOURS {
            f.move_to(i as f32, 0.0);
            f.line_to(i as f32, 1.0);
        }
        assert!(!f.overflowed, "恰好 {MAX_CONTOURS} 个轮廓不应超限");
    }

    /// `MAX_CONTOURS + 1` 个轮廓：必须超限。
    #[test]
    fn max_contours_plus_one_overflows() {
        let mut f = flat();
        for i in 0..=MAX_CONTOURS {
            f.move_to(i as f32, 0.0);
            f.line_to(i as f32, 1.0);
        }
        assert!(f.overflowed, "超过 {MAX_CONTOURS} 个轮廓应被拒收");
    }

    /// 超限后**继续消费回调也不崩**（ttf-parser 没有提前终止机制）。
    #[test]
    fn keeps_consuming_callbacks_after_overflow_without_panicking() {
        let mut f = flat();
        f.move_to(0.0, 0.0);
        for i in 0..(MAX_LINES * 2) {
            f.line_to(i as f32, 1.0);
        }
        f.quad_to(1.0, 1.0, 2.0, 2.0);
        f.curve_to(1.0, 1.0, 2.0, 2.0, 3.0, 3.0);
        f.close();
        assert!(f.overflowed);
        assert_eq!(
            f.lines.len(),
            MAX_LINES,
            "再多的回调也不应使 lines 继续增长"
        );
    }
}
