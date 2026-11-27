//! **`DirtyRegion` 的不变量**（属性测试，确定性伪随机）。
//!
//! 脏区是"局部重绘"的正确性基石：**少标一块 = 屏幕上留下残影，多标一块只是多画**。
//! 所以这里的断言全部朝"不能少"的方向：
//!
//! 1. 空矩形绝不入表（否则 `rects()` 里出现零面积块，后端 scissor 会退化）；
//! 2. 块数不超过 `MAX_RECTS`（超了必须退化成 `full`，不能无限堆积）；
//! 3. **覆盖性**：凡 `add` 过的矩形，其中每个像素中心都必须被最终脏区覆盖；
//! 4. `bounds()` 必须真正罩住所有脏块（这条曾因 f32 舍入被违反 —— 见 `bounds` 的注释）；
//! 5. `clear()` 之后既不是 `full` 也没有残块。

use y901_ui::{DirtyRegion, Rect};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() % 1000) as f32 / 1000.0 * (hi - lo)
    }
}

fn covered(d: &DirtyRegion, p: (f32, f32)) -> bool {
    if d.is_full() {
        return true;
    }
    d.rects()
        .iter()
        .any(|r| p.0 >= r.x && p.0 <= r.x + r.w && p.1 >= r.y && p.1 <= r.y + r.h)
}

#[test]
fn every_added_rect_stays_covered() {
    let mut rng = Lcg(0x5eed_1234);
    for case in 0..300 {
        let mut d = DirtyRegion::default();
        let mut added: Vec<Rect> = Vec::new();
        for step in 0..30 {
            let r = Rect::new(
                rng.f(-20.0, 380.0),
                rng.f(-20.0, 280.0),
                rng.f(0.0, 120.0),
                rng.f(0.0, 120.0),
            );
            d.add(r);
            if !r.is_empty() {
                added.push(r);
            }

            // 1) 不变量：空矩形绝不入表；块数不超过上限
            assert!(
                d.rects().iter().all(|x| !x.is_empty()),
                "[case{case} step{step}] 脏区里出现空矩形"
            );
            assert!(
                d.is_full() || d.len() <= DirtyRegion::MAX_RECTS,
                "[case{case} step{step}] 块数 {} 超过上限 {}",
                d.len(),
                DirtyRegion::MAX_RECTS
            );

            // 2) 覆盖性：**按像素中心**检查。
            //
            // 为什么不用矩形的角点：几何量是 f32，`Rect` 的右边界由 `x + w` 重算，
            // 合并后可能比来源小一个 ULP（3e-5）。后端在转 scissor 时是 `ceil` 向外取整，
            // 所以"最后一个像素有没有被覆盖"才是真正要保证的语义，
            // "数学角点是否精确在内"不是。用角点断言会得到随机红。
            for r in &added {
                let x0 = r.x.floor() as i32;
                let x1 = (r.x + r.w).ceil() as i32;
                let y0 = r.y.floor() as i32;
                let y1 = (r.y + r.h).ceil() as i32;
                for px in (x0..x1).step_by(4) {
                    for py in (y0..y1).step_by(4) {
                        let c = (px as f32 + 0.5, py as f32 + 0.5);
                        if !r.contains(y901_ui::Pos::new(c.0, c.1)) {
                            continue;
                        }
                        assert!(
                            covered(&d, c),
                            "[case{case} step{step}] 像素中心 {c:?} 在 add 过的矩形 {r:?} 内却\
                             没被覆盖；full={} rects={:?}",
                            d.is_full(),
                            d.rects()
                        );
                    }
                }
            }

            // 3) bounds() 必须覆盖所有脏块
            if !d.is_full() {
                if let Some(b) = d.bounds() {
                    for r in d.rects() {
                        assert!(
                            b.contains_rect(*r),
                            "[case{case} step{step}] bounds {b:?} 没盖住脏块 {r:?}"
                        );
                    }
                }
            }
        }

        // 4) clear 之后必须真的空
        d.clear();
        assert!(d.is_empty(), "[case{case}] clear 之后应无脏区");
        assert!(!d.is_full(), "[case{case}] clear 之后不应是 full");
    }
}
