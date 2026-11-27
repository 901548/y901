//! **图集打包的不变量**（属性测试，确定性伪随机）。
//!
//! `SkylinePacker` 的返回值直接决定"这个字形写到缓冲的哪一块"，算错就是**字形互相覆盖**
//! （画面出现别的字的笔画）或**写出缓冲**（越界写）。这里随机 200 张图集 × 80 次分配：
//!
//! 1. 每块必须完整落在 `[0, size)²` 内；
//! 2. 任意两块不得重叠（哪怕 1 像素都不行）；
//! 3. 零尺寸 / 超尺寸的请求必须被拒绝而不是静默越界。

use y901_render::atlas::SkylinePacker;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn packer_never_overlaps_or_leaves_the_atlas() {
    let mut rng = Lcg(0xc0ff_ee11);
    for case in 0..200 {
        let size = 32 + (rng.next() % 5) as u32 * 16; // 32..96
        let mut packer = SkylinePacker::new(size);
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        for step in 0..80 {
            let w = 1 + (rng.next() % 20) as u32;
            let h = 1 + (rng.next() % 20) as u32;
            let Some((x, y)) = packer.insert(w, h) else {
                continue;
            };
            // 1) 必须完整落在图集内
            assert!(
                x + w <= size && y + h <= size,
                "[case{case} step{step}] 越界: ({x},{y}) {w}x{h} in {size}"
            );
            // 2) 不得与任何已放置矩形重叠
            for &(px, py, pw, ph) in &placed {
                let overlap = x < px + pw && px < x + w && y < py + ph && py < y + h;
                assert!(
                    !overlap,
                    "[case{case} step{step}] 重叠: new ({x},{y},{w},{h}) vs 已放 ({px},{py},{pw},{ph})"
                );
            }
            placed.push((x, y, w, h));
        }
    }
}

#[test]
fn packer_rejects_oversized_and_zero_sized_requests() {
    let mut p = SkylinePacker::new(64);
    assert_eq!(p.insert(0, 10), None, "零宽必须拒绝");
    assert_eq!(p.insert(10, 0), None, "零高必须拒绝");
    assert_eq!(p.insert(65, 1), None, "超宽必须拒绝");
    assert_eq!(p.insert(1, 65), None, "超高必须拒绝（否则会写出缓冲）");
    assert!(p.insert(64, 64).is_some(), "正好整张应能放下");
}
