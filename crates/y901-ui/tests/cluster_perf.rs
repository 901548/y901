//! 光标移动的**分配量对照**（`#[ignore]` 剖析用例，不参与默认门禁计数）。
//!
//! 背景（PLAN 批次 6）：`TextBuffer` 的光标移动每帧调 3 次
//! `snap` / `prev_boundary` / `next_boundary`，而旧实现**每次调用都物化整份
//! `Vec<usize>` 边界表**（20 万字符文本 ≈ 1.6 MB × 3 次/帧）。
//! 现在这三个函数"只走一个簇"，不再分配。
//!
//! 本用例把两种算法并排跑，打印倍数，供下次重构前 A/B：
//! `cargo test -p y901-ui --test cluster_perf -- --ignored --nocapture`

use std::time::Instant;
use y901_ui::{cluster_boundaries, next_boundary, prev_boundary, snap};

/// 旧算法：物化边界表 + 二分（逐步照抄重构前的实现）。
fn old_snap(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if byte == 0 || byte >= text.len() {
        return byte.min(text.len());
    }
    let bounds = cluster_boundaries(text);
    match bounds.binary_search(&byte) {
        Ok(_) => byte,
        Err(idx) => bounds.get(idx.saturating_sub(1)).copied().unwrap_or(0),
    }
}

fn old_prev(text: &str, byte: usize) -> Option<usize> {
    let byte = old_snap(text, byte);
    let bounds = cluster_boundaries(text);
    bounds.iter().rev().copied().find(|b| *b < byte)
}

fn old_next(text: &str, byte: usize) -> Option<usize> {
    let byte = old_snap(text, byte);
    let bounds = cluster_boundaries(text);
    bounds.iter().copied().find(|b| *b > byte)
}

#[test]
#[ignore = "剖析用例：按需 --ignored 运行，打印新旧算法的耗时倍数"]
fn profile_cluster_walk_cost() {
    // 一行 200k 字符：右键移动 1000 次（真实编辑器里按住右键会连续触发）。
    let text = "x".repeat(200_000);
    let moves = 1000;

    let t0 = Instant::now();
    let mut pos = 0usize;
    for _ in 0..moves {
        pos = next_boundary(&text, pos).unwrap_or(text.len());
        let _ = snap(&text, pos);
        let _ = prev_boundary(&text, pos);
    }
    let new_cost = t0.elapsed();

    let t1 = Instant::now();
    let mut pos2 = 0usize;
    for _ in 0..moves {
        pos2 = old_next(&text, pos2).unwrap_or(text.len());
        let _ = old_snap(&text, pos2);
        let _ = old_prev(&text, pos2);
    }
    let old_cost = t1.elapsed();

    assert_eq!(pos, pos2, "两种算法必须给出同一个位置");
    let ratio = old_cost.as_secs_f64() / new_cost.as_secs_f64().max(1e-9);
    println!(
        "[cluster-perf] 新（按簇游走）={:?}  旧（物化 Vec）={:?}  倍数={:.1}x  ({moves} 次移动 / {} 字符)",
        new_cost,
        old_cost,
        ratio,
        text.len()
    );
    // 只做"别退化"的下限断言：新实现必须快于旧实现（宽松，避免 CI 抖动）。
    assert!(
        ratio > 1.2,
        "新实现没有比物化版本更快（{ratio:.2}x）：可能有人把边界表加回来了"
    );
}
