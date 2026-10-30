//! 16ms 延迟窗口的实测：动作返回的 rev 与随后 get_tree 看到的 rev 是否一致。
//!
//! 关心的**不是**"状态被别人改了还硬用旧 rev"（那本来就该 409），
//! 而是"Agent 刚做完动作，拿到的 rev 是否已经过期" —— 这才是 16ms 延迟的真实代价。
//!
//! 默认随 CI 运行（进程内直调，不经过 TCP）。
//! ```text
//! cargo test -p y901-widgets --test revision_latency -- --nocapture
//! ```

use y901_agent::{handle_call, ApiCall, ApiError, Policy};
use y901_ui::{Flex, FlexItem, Json, Size, TextMetrics, TextStyle, UiTree};
use y901_widgets::TextInput;

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn tree() -> UiTree {
    let input = TextInput::new("field").flex_item(FlexItem::height(24.0));
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(input))));
    tree.layout(Size::new(300.0, 100.0), &Mono);
    tree
}

fn rev_in_tree(view: &Json, id: &str) -> u64 {
    fn find<'a>(node: &'a Json, id: &str) -> Option<&'a Json> {
        if node.get("id").and_then(|v| v.as_str()) == Some(id) {
            return Some(node);
        }
        match node.get("children") {
            Some(Json::Arr(k)) => k.iter().find_map(|c| find(c, id)),
            _ => None,
        }
    }
    find(view, id)
        .and_then(|n| n.get("revision"))
        .and_then(|v| v.as_f64())
        .map(|f| f as u64)
        .unwrap_or(u64::MAX)
}

#[test]
fn action_rev_is_never_stale() {
    let mut tree = tree();
    let policy = Policy::default();
    let mut stale = 0usize;
    let mut conflicts = 0usize;
    let iters = 1000;

    for i in 0..iters {
        // 1) 做一次动作，拿到返回值里的 rev
        let resp = handle_call(
            &mut tree,
            &policy,
            ApiCall::Invoke {
                id: "field".into(),
                action: "set_text".into(),
                args: Json::obj().set("text", format!("v{i}")),
                if_rev: None,
            },
        )
        .expect("invoke");
        let rev_from_action = resp.get("rev").and_then(|v| v.as_f64()).unwrap_or(-1.0) as u64;

        // 2) 紧接着读树（模拟 Agent 的下一步观察），比较 rev
        let view = handle_call(&mut tree, &policy, ApiCall::Tree).expect("tree");
        let rev_in_view = rev_in_tree(&view, "field");
        if rev_from_action != rev_in_view {
            stale += 1;
        }

        // 3) 反向用例：用**上一轮**的 rev 去 invoke，应当 409（这是正确行为）
        if i > 0 {
            let r = handle_call(
                &mut tree,
                &policy,
                ApiCall::Invoke {
                    id: "field".into(),
                    action: "set_text".into(),
                    args: Json::obj().set("text", "x".to_string()),
                    if_rev: Some(rev_from_action.saturating_sub(1)),
                },
            );
            if matches!(r, Err(ApiError::Conflict { .. })) {
                conflicts += 1;
            }
        }
    }

    let stale_rate = stale as f64 / iters as f64 * 100.0;
    let conflict_rate = conflicts as f64 / (iters - 1) as f64 * 100.0;
    eprintln!(
        "[latency] iters={iters} stale={stale} ({stale_rate:.2}%) conflicts={conflicts} ({conflict_rate:.1}%)"
    );

    assert_eq!(
        stale, 0,
        "动作返回的 rev 不得与随后 get_tree 的 rev 不一致（stale {stale_rate:.2}%）"
    );
    // 用过期 rev 必须被拒——这是乐观锁的本职，不是延迟问题
    assert_eq!(conflicts, iters - 1, "过期 rev 必须 100% 返回 409");
}

#[test]
fn layout_is_not_required_between_action_and_read() {
    // 故意不调用 layout：验证 refresh 由 handle_call → revision_of 触发
    let mut tree = tree();
    let policy = Policy::default();

    let before = handle_call(&mut tree, &policy, ApiCall::Tree)
        .map(|v| rev_in_tree(&v, "field"))
        .unwrap_or(0);

    handle_call(
        &mut tree,
        &policy,
        ApiCall::Invoke {
            id: "field".into(),
            action: "set_text".into(),
            args: Json::obj().set("text", "hello".to_string()),
            if_rev: None,
        },
    )
    .expect("invoke");

    let after = handle_call(&mut tree, &policy, ApiCall::Tree)
        .map(|v| rev_in_tree(&v, "field"))
        .unwrap_or(0);

    assert!(
        after > before,
        "不调用 layout 时，handle_call 内部也必须刷新语义（before={before} after={after}）"
    );
}
