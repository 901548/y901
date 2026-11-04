//! 字形 mipmap 链与 LOD（细节层次）选择。
//!
//! 为什么需要它：字形一旦光栅化就是固定尺寸位图，而 UI 动画里缩放是连续变化的。
//! 每帧重新光栅化的代价是 **20–50µs / 字形**（轮廓加载 + 扁平化 + 填充 + 抗锯齿），
//! 20 个字形 × 60fps 就会吃掉 24–60ms/帧；而且每个不同的缩放值都是新的图集条目，
//! 会迅速把打包器填满并触发整图重建。
//!
//! 这里的做法：为每个字形预生成 3 级位图（1×、1/2、1/4），采样时按缩放比选级 —— `O(1)`。
//!
//! **只对缩小方向生成**：放大（`s > 1`）直接走原始位图路径，不生成 mip，
//! 所以图集代价是 `1 + 1/4 + 1/16 = 1.3125×`，而不是全方向的 `1.33×`。

/// 一级 mipmap：一张覆盖率位图。
///
/// 覆盖率是 `0..=255` 的单通道数据（与字形图集里的 `rgb` 通道同义）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    /// 位图宽度（像素）。
    pub width: u32,
    /// 位图高度（像素）。
    pub height: u32,
    /// 覆盖率，长度 = `width * height`，行优先。
    pub coverage: Vec<u8>,
}

impl MipLevel {
    /// 用给定尺寸与覆盖率数据建一级。
    ///
    /// 若 `coverage` 长度不足会**补零**而不是 panic —— 与框架其他"非法输入不炸 UI"的约定一致。
    pub fn new(width: u32, height: u32, coverage: Vec<u8>) -> Self {
        let need = (width as usize) * (height as usize);
        let mut coverage = coverage;
        coverage.resize(need, 0);
        Self {
            width,
            height,
            coverage,
        }
    }

    /// 墨迹覆盖率：非零像素占比（`0.0..=1.0`）。
    ///
    /// 这是 LOD 策略里"复杂字形多降一级"的依据：笔画密的字（如「量」）缩小后更容易粘连，
    /// 而稀疏字（如「一」）缩小后仍然清晰。
    pub fn ink_ratio(&self) -> f32 {
        let total = self.coverage.len();
        if total == 0 {
            return 0.0;
        }
        let inked = self.coverage.iter().filter(|c| **c > 0).count();
        inked as f32 / total as f32
    }

    /// 用 2×2 盒式滤波从本层降采样出一半尺寸的下一层。
    ///
    /// 存在的意义是**对照组**：设计文档承诺"每级重新光栅化优于盒式滤波"，
    /// 而这句话必须被测量，不能只靠断言。测试用它与真实光栅化结果比 MAD。
    pub fn downsample_box(&self) -> Self {
        let w = (self.width / 2).max(1);
        let h = (self.height / 2).max(1);
        let mut out = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut count = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = x * 2 + dx;
                        let sy = y * 2 + dy;
                        if sx < self.width && sy < self.height {
                            sum += self.coverage[(sy * self.width + sx) as usize] as u32;
                            count += 1;
                        }
                    }
                }
                out[(y * w + x) as usize] = if count == 0 { 0 } else { (sum / count) as u8 };
            }
        }
        Self {
            width: w,
            height: h,
            coverage: out,
        }
    }
}

/// 一个字形（或一张图片）的多级位图。
///
/// `levels[0]` 是原始尺寸，`levels[k]` 是 `1/2^k`。**只用于缩小方向**：
/// 放大（`scale >= 1`）时永远选 `levels[0]`。
#[derive(Clone, Debug, Default)]
pub struct MipChain {
    levels: Vec<MipLevel>,
}

impl MipChain {
    /// 用一张原始位图建立只有 L0 的链。
    pub fn new(base: MipLevel) -> Self {
        Self { levels: vec![base] }
    }

    /// 追加一级（调用方负责保证尺寸递减）。
    pub fn push(&mut self, level: MipLevel) {
        self.levels.push(level);
    }

    /// 级数（含 L0）。空链为 0。
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// 是否没有任何级。
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// 取某一级；越界返回 `None`（调用方据此回退到 L0）。
    pub fn get(&self, level: usize) -> Option<&MipLevel> {
        self.levels.get(level)
    }

    /// 最粗一级的下标（空链返回 0）。
    pub fn max_level(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }

    /// 按缩放比与墨迹密度选级。
    ///
    /// 策略（设计文档 §2.2 的 A + B）：
    ///
    /// 1. `scale >= 1.0` → **永远是 0**。放大不生成 mip，走原始位图路径；
    ///    这一步也是"s = 1.0 逐像素不变"的实现保证。
    /// 2. 否则 `base = floor(log2(1 / scale))` —— 取**向下取整**而不是四舍五入：
    ///    宁可多保留细节，也不要过早降到粗层（粗层的笔画断裂是不可逆的）。
    /// 3. `ink > 0.25` 时**多降一级**：密集字形缩小后笔画会粘连，粗层反而更均匀。
    /// 4. 结果夹到 `0..=max_level`，不越界。
    ///
    /// 非有限或非正的 `scale` 一律当作 1.0 处理（返回 0），因为那是调用方的错误，
    /// 按"不缩放"处理最安全。
    pub fn level_for(&self, scale: f32, ink: f32) -> usize {
        level_for(scale, ink, self.max_level())
    }


    /// 用盒式滤波逐级生成到 `levels` 级（仅用于**对照组**与测试）。
    ///
    /// 生产路径不这么生成：字形应由 TTF 在小尺寸下重新光栅化（见设计文档 §3.4）。
    pub fn with_box_levels(base: MipLevel, levels: usize) -> Self {
        let mut chain = Self::new(base);
        while chain.len() < levels.max(1) {
            let next = chain.levels.last().map(|l| l.downsample_box());
            match next {
                Some(level) => chain.push(level),
                None => break,
            }
        }
        chain
    }

    /// 两张同尺寸位图的平均绝对差（`0.0..=1.0`）——质量度量的最小实现。
    ///
    /// 为什么用 MAD 而不是 SSIM：SSIM 需要窗口化统计与更多实现细节，
    /// 而这里要回答的问题只是"哪条生成路线更接近理想结果"，MAD 足够且可解释。
    /// 尺寸不一致时返回 `None`（调用方不该拿不同尺寸比）。
    pub fn mad(a: &MipLevel, b: &MipLevel) -> Option<f32> {
        if a.width != b.width || a.height != b.height || a.coverage.len() != b.coverage.len() {
            return None;
        }
        if a.coverage.is_empty() {
            return Some(0.0);
        }
        let sum: u64 = a
            .coverage
            .iter()
            .zip(b.coverage.iter())
            .map(|(x, y)| x.abs_diff(*y) as u64)
            .sum();
        Some(sum as f32 / (255.0 * a.coverage.len() as f32))
    }
}


/// 按缩放比与墨迹密度选级（自由函数版，不需要构造链）。
///
/// `max_level` 是**可用的最粗一级**（链上已有的级数 - 1）。返回值保证 `<= max_level`，
/// 因此调用方可以安全地用它索引 `mips[max_level]`。
///
/// 规则与 [`MipChain::level_for`] 完全一致，此处不重复解释，只给边界速查：
/// `1.0 -> 0`、`0.75 -> 0`、`0.5 -> 1`（密字形 2）、`0.25 -> 2`、`NaN -> 0`。
pub fn level_for(scale: f32, ink: f32, max_level: usize) -> usize {
    if !(scale.is_finite() && scale > 0.0) || scale >= 1.0 {
        return 0;
    }
    let halvings = (1.0 / scale).log2().floor();
    let mut level = if halvings.is_finite() && halvings > 0.0 {
        halvings as usize
    } else {
        0
    };
    // 只从 **level >= 2** 开始才多降一级。
    //
    // 为什么：幂级 0 对应"s >= 0.5 的基础选择"。若从 0 往下降一级，
    // 0.75 会用 1/2 位图去画 11.25px 的字——那是**放大采样**，比原图更糊。
    // 密字形的优势只在"已经要缩小到半尺寸以下"时才成立。
    if ink > 0.25 && level >= 2 {
        level += 1;
    }
    level.min(max_level)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一条 3 级链（L0 8×8、L1 4×4、L2 2×2），墨迹随便给。
    fn chain3() -> MipChain {
        let l0 = MipLevel::new(8, 8, vec![200; 64]);
        let l1 = MipLevel::new(4, 4, vec![200; 16]);
        let l2 = MipLevel::new(2, 2, vec![200; 4]);
        MipChain {
            levels: vec![l0, l1, l2],
        }
    }

    #[test]
    fn level_zero_at_scale_one_regardless_of_ink() {
        let c = chain3();
        assert_eq!(c.level_for(1.0, 0.0), 0, "s = 1 必须走原图（逐像素不变）");
        assert_eq!(c.level_for(1.0, 0.9), 0, "墨迹密度不应影响 s = 1");
        assert_eq!(c.level_for(1.8, 0.9), 0, "放大方向不生成 mip，永远 L0");
    }

    #[test]
    fn level_zero_for_mild_shrink() {
        let c = chain3();
        // 0.75 → log2(1/0.75) = 0.415 → floor 0
        assert_eq!(c.level_for(0.75, 0.0), 0);
        assert_eq!(c.level_for(0.99, 0.0), 0);
    }

    #[test]
    fn level_one_for_half_scale_sparse_glyph() {
        let c = chain3();
        // 0.5 → log2(2) = 1 → floor 1；ink 低，不加级
        assert_eq!(c.level_for(0.5, 0.1), 1, "s = 0.5 稀疏字形取 L1");
    }

    #[test]
    fn dense_glyph_drops_one_extra_level() {
        let c = chain3();
        // 语义变更：密度加级从 `level >= 1` 改为 `level >= 2`。
        // 原因：0.5 的 base 是 1，加级会选 L2（L0/4）却要画到 L0×0.5 的尺寸上
        // —— 那是**放大采样**，实测让 'A' 的形状 MAD 从 0.090 恶化到 0.199。
        assert_eq!(c.level_for(0.5, 0.3), 1, "base level 1 不再加级");
        assert_eq!(c.level_for(0.5, 0.25), 1, "ink == 0.25 不触发加级");
        assert_eq!(c.level_for(0.5, 0.2501), 1);
        // base level 2（0.25）时加级仍生效，但会被链长夹住
        assert_eq!(c.level_for(0.25, 0.3), 2, "base 2 加级后被 clamp 到最粗级");
    }

    #[test]
    fn quarter_scale_reaches_the_coarsest_level() {
        let c = chain3();
        // 0.25 → log2(4) = 2 → floor 2
        assert_eq!(c.level_for(0.25, 0.0), 2);
    }

    #[test]
    fn very_small_scale_clamps_to_coarsest_level() {
        let c = chain3();
        assert_eq!(c.level_for(0.1, 0.0), 2, "低于 1/4 仍是最粗一级，不越界");
        // 密集字形在极小时也不能越界
        assert_eq!(c.level_for(0.1, 0.9), 2);
    }

    #[test]
    fn invalid_scale_is_treated_as_no_scaling() {
        let c = chain3();
        assert_eq!(c.level_for(f32::NAN, 0.5), 0, "NaN 按不缩放处理");
        assert_eq!(c.level_for(0.0, 0.5), 0, "0 不合法，按不缩放处理");
        assert_eq!(c.level_for(-1.0, 0.5), 0);
    }

    #[test]
    fn empty_chain_never_returns_out_of_range() {
        let c = MipChain::default();
        assert!(c.is_empty());
        assert_eq!(c.level_for(0.5, 0.9), 0);
        assert!(c.get(0).is_none());
    }

    #[test]
    fn box_downsample_halves_dimensions_and_averages() {
        // 2×2 全 100 → 1×1 应为 100（均值）
        let base = MipLevel::new(2, 2, vec![100, 100, 100, 100]);
        let down = base.downsample_box();
        assert_eq!((down.width, down.height), (1, 1));
        assert_eq!(down.coverage, vec![100]);

        // 一半 0 一半 255 → 均值 127/128
        let base = MipLevel::new(2, 2, vec![0, 255, 0, 255]);
        let down = base.downsample_box();
        assert_eq!(down.coverage, vec![127]);
    }

    #[test]
    fn mad_is_zero_for_identical_and_scales_with_difference() {
        let a = MipLevel::new(2, 2, vec![0, 0, 0, 0]);
        let b = MipLevel::new(2, 2, vec![0, 0, 0, 0]);
        assert_eq!(MipChain::mad(&a, &b), Some(0.0));

        let c = MipLevel::new(2, 2, vec![255, 255, 255, 255]);
        let mad = MipChain::mad(&a, &c).expect("同尺寸");
        assert!((mad - 1.0).abs() < 1e-6, "全黑 vs 全白 MAD 应为 1，实际 {mad}");

        // 尺寸不一致 → None（不该拿不同尺寸比）
        let d = MipLevel::new(1, 1, vec![0]);
        assert_eq!(MipChain::mad(&a, &d), None);
    }

    #[test]
    fn ink_ratio_counts_non_zero_pixels() {
        let l = MipLevel::new(2, 2, vec![0, 1, 0, 255]);
        assert!((l.ink_ratio() - 0.5).abs() < 1e-6);
        let empty = MipLevel::new(2, 2, vec![0; 4]);
        assert_eq!(empty.ink_ratio(), 0.0);
    }

    #[test]
    fn box_chain_has_requested_number_of_levels() {
        let base = MipLevel::new(8, 8, vec![128; 64]);
        let chain = MipChain::with_box_levels(base, 3);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain.get(0).map(|l| l.width), Some(8));
        assert_eq!(chain.get(1).map(|l| l.width), Some(4));
        assert_eq!(chain.get(2).map(|l| l.width), Some(2));
    }
}

/// 图片用的 LOD 选择：**只看缩放比，不看墨量**。
///
/// 与 `level_for`（字形用）的区别在于"真值来源"：
/// - 字形的每一级都是**在该字号下重新光栅化**得到的，所以"密集字形多降一级"
///   在小字号下有真实依据（笔画边缘占比上升）；
/// - 图片的每一级都是**同一张位图的 box 降采样**，没有"更精细的源"可挖，
///   所以没有 `ink` 这个维度。
///
/// 输入：`scale`（**实际绘制倍率**）、`max_level`。
/// 输出：目标级（未满足时由调用方向下回退）。
///
/// 为何与字形同样取 **floor**：0.5 是中点，选粗一级会把细节永久丢掉；
/// 图片丢细节的后果比字形更难察觉（没有"笔画断裂"这种强信号）。
pub fn level_for_image(scale: f32, max_level: usize) -> usize {
    if !(scale.is_finite() && scale > 0.0 && scale < 1.0) {
        return 0;
    }
    let lvl = (1.0 / scale).log2().floor().max(0.0);
    (lvl as usize).min(max_level)
}

#[cfg(test)]
mod image_lod_tests {
    use super::level_for_image;

    #[test]
    fn image_scale_one_uses_l0() {
        assert_eq!(level_for_image(1.0, 2), 0);
    }

    #[test]
    fn image_scale_three_quarters_uses_l0() {
        // floor(log2(1/0.75)) = floor(0.415) = 0
        assert_eq!(level_for_image(0.75, 2), 0);
    }

    #[test]
    fn image_scale_half_uses_l1() {
        assert_eq!(level_for_image(0.5, 2), 1);
    }

    #[test]
    fn image_scale_quarter_uses_l2() {
        assert_eq!(level_for_image(0.25, 2), 2);
    }

    #[test]
    fn image_invalid_scale_falls_back_to_l0() {
        assert_eq!(level_for_image(f32::NAN, 2), 0);
        assert_eq!(level_for_image(0.0, 2), 0);
        assert_eq!(level_for_image(-1.0, 2), 0);
        assert_eq!(level_for_image(2.0, 2), 0);
    }

    #[test]
    fn image_lod_is_clamped_to_available_levels() {
        // 1/16 会算出 level 4，但链只有 0..=2
        assert_eq!(level_for_image(1.0 / 16.0, 2), 2);
    }
}

/// 图片的 mip 链生成（box 降采样）。
///
/// 与字形的区别：图片源是**固定位图**，每级只能由上一级 box 降采样得到；
/// 字形能"重新光栅化"是因为源是无限精度的 TTF 轮廓。
///
/// 输入：基层（L0）像素与最多生成的级数（`levels` = 2 表示 L1+L2）。
/// 输出：链；尺寸已到 1px 的级会提前停止（不生成 0 宽的位图）。
pub fn image_mip_chain(base: MipLevel, levels: usize) -> MipChain {
    let mut chain = MipChain::new(base);
    while chain.len() <= levels {
        let prev = chain.get(chain.len() - 1).expect("刚推入的级");
        if prev.width <= 1 && prev.height <= 1 {
            break;
        }
        let next = prev.downsample_box();
        chain.push(next);
    }
    chain
}

#[cfg(test)]
mod image_mip_tests {
    use super::*;

    /// 用一张 2×2 四色图验证 box 降采样的像素值：
    /// L1 的每个像素应是对应 2×2 块的**平均值**，不是取一个点。
    #[test]
    fn box_downsample_is_average_of_four() {
        let base = MipLevel::new(
            2,
            2,
            vec![0, 100, 200, 255], // 四色
        );
        let chain = image_mip_chain(base, 1);
        assert_eq!(chain.len(), 2, "应生成 L0 + L1");
        let l1 = chain.get(1).expect("L1");
        assert_eq!((l1.width, l1.height), (1, 1));
        let avg = (0.0 + 100.0 + 200.0 + 255.0) / 4.0;
        assert!(
            (l1.coverage[0] as f32 - avg).abs() <= 1.0,
            "L1 应为四点平均值（~{avg}），实测 {}",
            l1.coverage[0]
        );
    }

    /// 已知形状：左半黑右半白的 4×4，L1 左半应接近 0、右半接近 255。
    /// 这是一个**可推导**的结果，不是拿 box filter 自己当真值。
    #[test]
    fn half_black_half_white_keeps_the_split() {
        let mut cov = vec![0u8; 16];
        for y in 0..4 {
            for x in 0..4 {
                cov[y * 4 + x] = if x < 2 { 0 } else { 255 };
            }
        }
        let chain = image_mip_chain(MipLevel::new(4, 4, cov), 1);
        let l1 = chain.get(1).expect("L1");
        assert_eq!((l1.width, l1.height), (2, 2));
        for y in 0..2 {
            assert!(l1.coverage[y * 2] <= 2, "左半应保持黑");
            assert!(l1.coverage[y * 2 + 1] >= 253, "右半应保持白");
        }
    }

    /// 尺寸小到 1px 时必须停止，不生成 0 宽位图（否则下一次降采样会除零）。
    #[test]
    fn stops_before_generating_zero_sized_levels() {
        let chain = image_mip_chain(MipLevel::new(1, 1, vec![7]), 2);
        assert_eq!(chain.len(), 1, "1×1 不应再生成 L1");
    }

    /// 1024² → L1 512²、L2 256²；储存 **1 + 1/4 + 1/16 = 1.3125×**。
    ///
    /// 注意：无穷级数列 1+1/4+1/16+… = **4/3 ≈ 1.333**，但只生成 L1+L2 时
    /// 只有 **1.3125**。我第一版把它当成 1.333 写进断言，测出 1.312 才发现算错了。
    #[test]
    fn storage_factor_is_about_one_third_extra() {
        let n = 1024 * 1024;
        let chain = image_mip_chain(MipLevel::new(1024, 1024, vec![0; n]), 2);
        assert_eq!(chain.len(), 3);
        assert_eq!((chain.get(1).unwrap().width, chain.get(2).unwrap().width), (512, 256));
        let total: usize = (0..chain.len()).map(|i| {
            let l = chain.get(i).unwrap();
            (l.width * l.height) as usize
        }).sum();
        let factor = total as f64 / n as f64;
        assert!((factor - 1.3125).abs() < 0.005, "存储倍数应为 1.3125（L0+L1+L2），实测 {factor:.4}");
    }
}
