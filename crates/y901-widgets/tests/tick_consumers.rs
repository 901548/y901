//! 源码扫描：声明了"每帧心跳需求"的组件必须实现 `wants_tick()`。
//!
//! 为什么需要这条测试：`tree.rs` 的广播门控只给 `wants_tick() == true` 的节点发 `Tick`
//! （默认 `false`）。组件里如果用了 `ctx.start_animation` / 在 `render` 里读
//! `ctx.animation(..)` / 靠 `UiEvent::Tick` 推进状态，却忘了实现 `wants_tick`，
//! 结果就是"代码看起来对、画面上永远不动"——Spinner 就曾这样静默失灵。

use std::fs;
use std::path::Path;

/// 需要每帧心跳的组件（新增此类组件时请登记在这里）。
const TICK_CONSUMERS: &[&str] = &["spinner", "skeleton", "combo_box", "text_input"];

fn widgets_src() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn tick_consumers_declare_wants_tick() {
    let src = widgets_src();
    for name in TICK_CONSUMERS {
        let path = src.join(format!("{name}.rs"));
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
        assert!(
            text.contains("fn wants_tick(&self) -> bool"),
            "{name}.rs 需要 Tick（动画/脉冲/光标），但没有实现 wants_tick —— \
             框架的广播门控不会给它发 Tick，运行时会静默不动"
        );
    }
}

#[test]
fn tick_consumers_really_use_tick_or_animation() {
    // 反向检查：登记在册的组件确实有"每帧需求"的证据，避免名单腐化。
    let src = widgets_src();
    for name in TICK_CONSUMERS {
        let text = fs::read_to_string(src.join(format!("{name}.rs"))).expect("read");
        let uses = text.contains("UiEvent::Tick")
            || text.contains("ctx.animation(")
            || text.contains("start_animation(");
        assert!(uses, "{name}.rs 登记为 Tick 消费者，但源码里找不到 Tick/动画使用点");
    }
}