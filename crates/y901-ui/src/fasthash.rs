//! 供**内部**索引表使用的快速哈希（FNV-1a）。
//!
//! ## 为什么需要它
//!
//! 布局与索引表的主键是 `u64`（由 [`crate::layout::id_key`] 预先把节点 id 哈希出来的），
//! 而 `std::collections::HashMap` 默认用 SipHash-1-3：对**已经被哈希过的 64 位键**
//! 再跑一遍密码学强度的混合函数，单次约 20–30ns。
//!
//! 实测（`crates/y901-ui/tests/layout_profile.rs`，11k 节点全量重排）：
//! 每节点要查 4–5 次表（`is_fresh` 的 contains + get、`parents.insert`、
//! `entries.insert`、`dirty.remove`），仅哈希就占掉冷布局的大头 ——
//! 宽树（1 个容器 + 10k 叶子）也有 **114ns/节点**，而叶子自身的工作几乎为零。
//!
//! 换成 FNV-1a 之后同样的键只需 1–2ns。**不是**放弃抗碰撞能力换性能：
//! 这些表全是进程内、键空间由我们自己生成（`u64`），不存在"外部输入构造碰撞"
//! 的攻击面；需要抗碰撞的场景（解析不可信输入）不在这里。
//!
//! ## 为什么不用外部 crate
//!
//! `rustc-hash` / `ahash` 都能做这件事，但本仓库的硬约束是"依赖面尽量小、
//! 全链纯 Rust 且可审计"。这里只需要 20 行，没必要为它增加一个依赖。

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// FNV-1a（64 位）的起始值。
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 的质数。
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 流式哈希器（字节流路径是标准 FNV-1a；`u64` 快路见 `write_u64`）。
///
/// 初值是 FNV 的 **offset basis**（不是 0）：这样 `write(b"")`、`write_u64` 的结果与
/// 标准 FNV-1a 一致，也不会出现"累加器恰好等于 0 时被误当成未初始化"的分支 ——
/// 那个分支让字节流变成**分块相关**，与"流式 FNV-1a"的承诺不符。
///
/// `write_u64` 走的是"键本来就是均匀的"这条快路径：
/// 直接异或再乘质数，不做逐字节循环。
#[derive(Clone)]
pub struct FnvHasher(u64);

impl Default for FnvHasher {
    fn default() -> Self {
        Self(FNV_OFFSET)
    }
}

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.0;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        self.0 = hash;
    }

    /// `u64` 键的快路：**一轮混合**，不是标准 FNV-1a 的 8 轮。
    ///
    /// 为什么可以不一样：本模块只被 `HashMap`/`HashSet` 用，只需要"分布好"，
    /// 不需要与标准 FNV-1a 一致（没有跨进程/跨版本的哈希值对外暴露）。
    /// 而键本身已经是 [`crate::layout::id_key`] 用完整 FNV-1a 算过的值 ——
    /// 再喂 8 轮只会多花 7 次乘法换不到任何分布收益。
    ///
    /// 先做一次 `n ^= n >> 32`：让**高位**也参与低位，避免"高位不同、低位相同"
    /// 的键挤进同一个桶。
    fn write_u64(&mut self, n: u64) {
        let mixed = n ^ (n >> 32);
        self.0 = (self.0 ^ mixed).wrapping_mul(FNV_PRIME);
    }

    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }
}

/// `FnvHasher` 的 `BuildHasher`。
pub type FnvBuildHasher = BuildHasherDefault<FnvHasher>;

/// 内部用：`u64` 键的快速映射表。
pub type FastMap<K, V> = HashMap<K, V, FnvBuildHasher>;
/// 内部用：`u64` 键的快速集合。
pub type FastSet<T> = HashSet<T, FnvBuildHasher>;

/// 建一张空的快速映射表。
pub fn fast_map<K, V>() -> FastMap<K, V> {
    FastMap::default()
}

/// 建一个空的快速集合。
pub fn fast_set<T>() -> FastSet<T> {
    FastSet::default()
}

/// 把一个**已经存在**的标准 `HashMap` 换成快速表（保留内容）。
pub fn fast_map_from<K, V>(map: HashMap<K, V>) -> FastMap<K, V>
where
    K: std::hash::Hash + Eq,
{
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_keys_same_bucket_regardless_of_insert_order() {
        let mut a = fast_map::<u64, u32>();
        let mut b = fast_map::<u64, u32>();
        for (i, k) in [7u64, 1, 99, 42].iter().enumerate() {
            a.insert(*k, i as u32);
        }
        for (i, k) in [42u64, 99, 1, 7].iter().enumerate() {
            b.insert(*k, 3 - i as u32);
        }
        assert_eq!(a, b, "同样的键值内容，插入顺序不应影响相等性");
    }

    #[test]
    fn distributes_sequential_keys() {
        // 顺序键（0..4096）不能全挤进同一个小桶集合：
        // 用 4096 个连续键的落桶分布来粗测分散度。
        use std::hash::BuildHasher;
        let bh = FnvBuildHasher::default();
        let mut buckets = std::collections::HashSet::new();
        for i in 0..4096u64 {
            buckets.insert(bh.hash_one(i) % 1024);
        }
        assert!(
            buckets.len() > 900,
            "4096 个连续键应散进几乎全部 1024 个桶，实际 {}",
            buckets.len()
        );
    }

    #[test]
    fn u64_and_byte_paths_agree_on_construction() {
        // 不承诺两条路径结果相同，但都必须是确定性的（同样输入 → 同样输出）。
        let mut h1 = FnvHasher::default();
        h1.write_u64(123);
        let mut h2 = FnvHasher::default();
        h2.write_u64(123);
        assert_eq!(h1.finish(), h2.finish());
    }

    /// `write_u64` 快路必须把**高位**也混进低位。
    ///
    /// 回归目标：旧实现只做 `(state ^ n) * PRIME`，键的高位不参与低位，
    /// 于是"高位不同、低位相同"的键（例如同一批 id 的哈希值只差高位）会挤进同一个小桶。
    #[test]
    fn u64_fast_path_mixes_high_bits_into_low_bits() {
        use std::hash::BuildHasher;
        let bh = FnvBuildHasher::default();
        // 这些键的低 32 位相同、高 32 位不同：旧的单轮混合会给它们相同的低位。
        let keys: Vec<u64> = (0..256u64).map(|i| (i << 32) | 0x1234).collect();
        let low: std::collections::HashSet<u64> =
            keys.iter().map(|k| bh.hash_one(k) % 1024).collect();
        assert!(
            low.len() > 200,
            "只有高 32 位不同的 256 个键必须散开，实际落进 {} 个桶",
            low.len()
        );
        // 确定性不变
        assert_eq!(bh.hash_one(42u64), bh.hash_one(42u64));
    }

    /// **空输入必须是标准 FNV offset basis**。
    ///
    /// 旧实现用"状态 == 0 表示未初始化"，于是 `write(b"")` 收在 0、而真实 FNV-1a
    /// 收在 offset basis；更糟的是"累加器恰好为 0"会被当成重新开始，
    /// 使字节流变成**分块相关**（同一串字节分两次喂进去结果不同）。
    #[test]
    fn empty_input_is_the_offset_basis() {
        use std::hash::Hasher;
        let mut h = FnvHasher::default();
        h.write(b"");
        assert_eq!(h.finish(), FNV_OFFSET);

        // 分块无关：一次喂 "ab" 与分两次喂 "a"、"b" 必须相同
        let mut one = FnvHasher::default();
        one.write(b"ab");
        let mut two = FnvHasher::default();
        two.write(b"a");
        two.write(b"b");
        assert_eq!(one.finish(), two.finish(), "流式哈希不能分块相关");
    }
}
