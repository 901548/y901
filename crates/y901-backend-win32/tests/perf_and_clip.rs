#![cfg(windows)]

//! 性能与裁剪回归：主循环空闲必须休眠；客户区必须有基座裁剪。
//!
//! 本机无 GPU，无法实测帧时间，所以用结构断言守住两个修复点，
//! 并用 `clip_quad` 的单元测试证明"越界字形会被裁掉"。

#[test]
fn runtime_idles_when_nothing_to_do() {
    // 主循环必须休眠，否则空闲时 100% CPU 空转。
    // 重构后休眠分层了：这里跨两个文件验证（比旧版只看一个文件更强）。
    let common = include_str!("../../y901-runtime/src/runtime.rs");
    let win32 = include_str!("../src/window.rs");

    assert!(
        common.contains("window.wait_events(wait_ms)"),
        "公共循环必须通过 wait_events 取事件"
    );

    let fn_start = win32
        .find("fn wait_events(")
        .expect("Win32 必须实现 wait_events");
    let body = &win32[fn_start..];
    let body_end = body.find("fn viewport(").unwrap_or(body.len());
    let body = &body[..body_end];
    assert!(
        body.contains("wait_message("),
        "Win32 的 wait_events 必须调 wait_message 休眠\n{body}"
    );

    // “忙时不睡”的语义：在赋值 `wait_ms` 的那段代码里，
    // 必须同时出现 `was_busy`（判忙）与 `idle_wait_ms()`（取等待上限）。
    //
    // 为何不直接匹配整行 `if was_busy { 0 } else { idle_wait_ms() }`：
    // rustfmt 会把它折成多行，那样断言会因为“没有独立一行”而假红。
    // 只要这两个 token 还在同一段赋值里，语义就没变。
    // 从计算起点开始取：`tick_due_ms`（含 idle_wait_ms）在 `let wait_ms` 之前。
    let wait_start = common
        .find("let tick_due_ms")
        .or_else(|| common.find("let wait_ms"))
        .expect("run_loop 必须计算 wait_ms");
    // 取足够长的片段：如今 wait_ms 由 tick_due_ms（帧预算与
    // Tick 到期时间的较小值）计算，比原来多几行。
    let wait_expr = &common[wait_start..(wait_start + 900).min(common.len())];
    assert!(
        wait_expr.contains("was_busy"),
        "wait_ms 的计算必须参考 was_busy（忙时不睡），实测片段：{wait_expr}"
    );
    assert!(
        wait_expr.contains("idle_wait_ms()"),
        "wait_ms 的闲档必须来自 idle_wait_ms()，实测片段：{wait_expr}"
    );
    // 忙档现在分两种：**自节流后端（D3D11 Present(1) vsync）不睡**，
    // 否则保留 16ms 帧预算（软后端靠它限速）。若忙档又回到无条件 `0`，
    // 软后端会空转；若取消自节流分支，D3D11 会回到 30fps 双重节流。
    //
    // **必须锚定在 `let wait_ms` 之后**：前面 `tick_due_ms` 的分支里也有一个
    // `from_millis(16)`，若只用 `wait_expr.contains("16")`，即使把 `wait_ms` 的忙档改回 `0`
    // 测试仍会通过 —— 那是一条空转断言。
    let after_wait = wait_expr
        .split("let wait_ms")
        .nth(1)
        .expect("wait_expr 应包含 let wait_ms（否则断言锚点已失效）");
    // 忙档：**帧预算补偿** —— 16ms 减去上一帧耗时，而不是固定睡 16ms。
    // 两条回归都要被抳住：
    //   1) 回到"固定 16ms" → 帧周期被拉长（40fps）；
    //   2) 回到"忙档无条件 0" → 软后端空转烧 CPU。
    assert!(
        after_wait.contains("saturating_sub"),
        "wait_ms 的忙档应做帧预算补偿（锚定 let wait_ms 之后），实测：{after_wait}"
    );
    assert!(
        after_wait.contains("last_frame_ms"),
        "wait_ms 的忙档必须参考上一帧耗时，实测：{after_wait}"
    );

    // Tick 节拍必须参考 `wants_tick` 订阅者，否则闲置动画只能 10fps。
    // 从 `has_tick_consumers` 起取：那一行才是读 `tick_subscribers()` 的地方，
    // 而 `tick_period` 的 if 里用的是本地变量名。
    let tick_start = common
        .find("let has_tick_consumers")
        .or_else(|| common.find("let tick_period"))
        .expect("run_loop 必须计算 tick_period");
    let tick_expr = &common[tick_start..(tick_start + 320).min(common.len())];
    assert!(
        tick_expr.contains("tick_subscribers"),
        "tick_period 的忙档必须包含 tick_subscribers，实测：{tick_expr}"
    );
}

/// 后端的基座裁剪必须是客户区：`begin()` 里 push 一个覆盖视口的 clip。
#[test]
fn backend_seeds_client_area_clip() {
    let src = include_str!("../src/backend/mod.rs");
    let start = src.find("pub fn begin(").expect("begin 必须存在");
    let body = &src[start..];
    let end = body[1..]
        .find("\n    pub ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];
    let compact: String = body.split_whitespace().collect();
    assert!(
        compact.contains("self.state.push_clip("),
        "begin() 必须压入客户区裁剪（否则内容会画到窗口外）：\n{body}"
    );
    assert!(
        body.contains("self.state.clear_clips()"),
        "每帧必须重置裁剪栈"
    );
}

/// 字形必须走 `clip_quad` 裁剪（`clip_of` 只覆盖 fill_rect）。
#[test]
fn glyphs_are_clipped_by_quad() {
    let src = include_str!("../src/backend/mod.rs");
    let start = src.find("fn push_glyph(").expect("push_glyph 必须存在");
    let body = &src[start..];
    let end = body[1..]
        .find("\n    fn ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];
    assert!(
        body.contains("clip_quad("),
        "字形必须按裁剪框缩放并重映射 UV：\n{body}"
    );
    assert!(
        body.contains("let Some(quad) = clip_quad("),
        "裁剪为空时必须丢弃该字形（`None` 分支直接 return）"
    );
}
