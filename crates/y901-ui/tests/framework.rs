//! 框架层集成测试。
//!
//! 重点验证内核三条硬指标：
//! 1. 2000 节点树在相同约束下第二次布局，`measure` 调用次数为 **0**；
//! 2. 无界约束返回**有限**内容尺寸，不产生 inf / NaN；
//! 3. 脏区能整棵剪掉不相交的子树，悬停只弄脏两个节点。

use std::sync::mpsc::channel;
use std::time::Duration;

use y901_ui::layout::{flex_measure, Flex, LayoutCache};
use y901_ui::{
    Axis, Color, Command, Constraints, EventPhase, EventResult, FlexItem, Json, Pos, Rect, RenderCtx,
    Role, Size, TextStyle, TextMetrics, UiEvent, UiTree, Widget,
};

thread_local! {
    /// 每个测试线程独立计数，避免并行测试互相干扰。
    static MEASURE_CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn reset_measures() {
    MEASURE_CALLS.with(|c| c.set(0));
}

fn measure_count() -> u32 {
    MEASURE_CALLS.with(|c| c.get())
}

fn bump_measure() {
    MEASURE_CALLS.with(|c| c.set(c.get() + 1));
}

// ---------- 假后端 ----------

struct MonoMetrics;

impl TextMetrics for MonoMetrics {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 记录绘制指令的后端（本文件只关心计数，不保留内容）
#[derive(Default)]
struct Recorder {}

impl TextMetrics for Recorder {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        MonoMetrics.measure_text(t, s)
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        MonoMetrics.line_height(s)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Recorder {
    fn size(&self) -> Size {
        Size::new(800.0, 600.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

// ---------- 测试用叶子 ----------

struct Leaf {
    id: String,
    version: u64,
    rect: Rect,
    item: FlexItem,
}

impl Leaf {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            version: 0,
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }

    fn layout_version(&self) -> u64 {
        self.version
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    /// 测试夹具：Leaf 需要能被 set_focus（新语义要求目标 `focusable()`）。
    fn focusable(&self) -> bool {
        true
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        bump_measure();
        c.constrain(Size::new(10.0, 20.0))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 只记录"我画了"，用一个标记 id 进入 texts 不方便，这里用 rect 占位
        ctx.fill_rect(self.rect, Color::WHITE);
    }
}

/// 会记录自己被画过的叶子。
struct Tracked {
    inner: Leaf,
    seen: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl Widget for Tracked {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn layout_version(&self) -> u64 {
        self.inner.version
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        self.inner.measure(m, c, cache)
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.inner.arrange(r, m, cache)
    }
    fn rect(&self) -> Rect {
        self.inner.rect()
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        self.seen.borrow_mut().push(self.inner.id().to_string());
        self.inner.render(ctx);
    }
}

// ---------- 1. 2000 节点：第二次布局 measure = 0 ----------

fn build_2000() -> UiTree {
    // 20 组 × 100 个叶子 = 2000 个叶子 + 21 个容器
    let mut root = Flex::column("root");
    for g in 0..20 {
        let mut row = Flex::row(format!("row{g}")).flex_item(FlexItem::height(20.0));
        for i in 0..100 {
            row = row.push(Box::new(Leaf::new(&format!("leaf{g}_{i}"))));
        }
        root = root.push(Box::new(row));
    }
    UiTree::new(Box::new(root))
}

#[test]
fn second_layout_measures_zero() {
    reset_measures();
    let mut tree = build_2000();

    tree.layout(Size::new(800.0, 600.0), &MonoMetrics);
    let first = measure_count();
    assert_eq!(first, 2000, "首帧应测 2000 个叶子，实际 {first}");
    assert!(!tree.layout_stats().skipped);

    reset_measures();
    tree.layout(Size::new(800.0, 600.0), &MonoMetrics);
    let stats = tree.layout_stats();
    let second = measure_count();
    assert_eq!(second, 0, "相同约束下第二次布局必须 0 次 measure");
    assert!(stats.skipped, "整次布局应被跳过");
}

#[test]
fn changing_one_leaf_only_remeasures_its_path() {
    reset_measures();
    let mut tree = build_2000();
    tree.layout(Size::new(800.0, 600.0), &MonoMetrics);

    // 让某个叶子"尺寸变了"
    if let Some(node) = tree.find_mut("leaf7_42") {
        // 通过 downcast 拿不到具体类型，这里用事件让树自己标记脏
        let _ = node;
    }
    // 直接对布局缓存标脏（模拟 EventResult::Relayout 的效果）
    tree.mark_layout_dirty("leaf7_42");

    reset_measures();
    tree.layout(Size::new(800.0, 600.0), &MonoMetrics);
    let calls = measure_count();

    // 该叶子 + row7 + root ≈ 3，绝不应该重新测 2000 个
    assert!(calls <= 4, "只应重测脏路径，实际 {calls}");
    assert!(calls >= 1);
}

// ---------- 2. 无界约束 ----------

struct ScrollLike {
    id: String,
    child: Box<dyn Widget>,
    rect: Rect,
}

impl Widget for ScrollLike {
    fn id(&self) -> &str {
        &self.id
    }

    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        // 主轴无界：问内容到底多高
        let content = cache.measure(
            self.child.as_mut(),
            m,
            Constraints::unbounded_main(Axis::Vertical, c.max),
        );
        assert!(content.h.is_finite(), "内容高度必须是有限值");
        // 视口只取 100 高
        c.constrain(Size::new(content.w, content.h.min(100.0)))
    }

    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        // 内容坐标系：子节点从视口原点向下排，不因为滚动产生巨大坐标
        self.child
            .arrange(Rect::new(rect.x, rect.y, rect.w, rect.h), m, cache);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        std::slice::from_ref(&self.child)
    }

    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

#[test]
fn unbounded_measure_returns_finite_content() {
    let mut column = Flex::column("content");
    for i in 0..50 {
        column = column.push(Box::new(Leaf::new(&format!("row{i}"))));
    }
    let mut scroll = ScrollLike {
        id: "scroll".into(),
        child: Box::new(column),
        rect: Rect::ZERO,
    };

    let mut cache = LayoutCache::new();
    let size = scroll.measure(
        &MonoMetrics,
        Constraints::unbounded_main(Axis::Vertical, Size::new(300.0, 0.0)),
        &mut cache,
    );

    assert!(size.h.is_finite());
    assert_eq!(size.h, 100.0, "视口应被限制在 100");
    // 内容真实高度 = 50 × 20
    let content = cache.measure(
        scroll.child.as_mut(),
        &MonoMetrics,
        Constraints::unbounded_main(Axis::Vertical, Size::new(300.0, 0.0)),
    );
    assert_eq!(content.h, 1000.0, "内容尺寸必须是真实值，不是 inf");
}

#[test]
fn infinite_constraints_never_produce_nan() {
    let mut cache = LayoutCache::new();
    let mut leaf = Leaf::new("x");
    let c = Constraints {
        max: Size::new(f32::INFINITY, f32::INFINITY),
        min: Size::ZERO,
        unbounded: y901_ui::Axis2::BOTH,
    };
    let size = leaf.measure(&MonoMetrics, c, &mut cache);
    assert!(size.w.is_finite() && size.h.is_finite());
}

// ---------- 3. 脏区剪枝 ----------

#[test]
fn dirty_region_prunes_untouched_subtrees() {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mk = |id: &str| {
        Box::new(Tracked {
            inner: Leaf::new(id),
            seen: seen.clone(),
        }) as Box<dyn Widget>
    };

    let root = Flex::row("root")
        .push(Box::new(
            Flex::column("left")
                .flex_item(FlexItem::fill())
                .push(mk("a"))
                .push(mk("b")),
        ))
        .push(Box::new(
            Flex::column("right")
                .flex_item(FlexItem::fill())
                .push(mk("c"))
                .push(mk("d")),
        ));

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 200.0), &MonoMetrics);

    // 首帧：全画
    let mut rec = Recorder::default();
    tree.render(&mut rec);
    assert_eq!(seen.borrow().len(), 4, "首帧应画出全部 4 个叶子");

    seen.borrow_mut().clear();
    tree.invalidate("a");
    tree.render(&mut rec);

    let drawn = seen.borrow().clone();
    assert!(drawn.contains(&"a".to_string()));
    assert!(!drawn.contains(&"c".to_string()), "右半树应被剪掉");
    assert!(!drawn.contains(&"d".to_string()), "右半树应被剪掉");
}

#[test]
fn hover_only_dirties_two_nodes() {
    let root = Flex::row("root")
        .push(Box::new(
            Leaf::new("a").flex_item(FlexItem {
                width: y901_ui::Dimension::Px(100.0),
                height: y901_ui::Dimension::Flex(1.0),
            }),
        ))
        .push(Box::new(
            Leaf::new("b").flex_item(FlexItem {
                width: y901_ui::Dimension::Px(100.0),
                height: y901_ui::Dimension::Flex(1.0),
            }),
        ));

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &MonoMetrics);
    // 消费首帧脏区
    let mut rec = Recorder::default();
    tree.render(&mut rec);

    tree.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(50.0, 50.0),
    });
    assert_eq!(tree.hovered(), Some("a"));
    assert_eq!(tree.dirty().len(), 1, "只有 a 变脏");

    tree.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(150.0, 50.0),
    });
    assert_eq!(tree.hovered(), Some("b"));
    assert_eq!(tree.dirty().len(), 2, "旧节点 a + 新节点 b");

    // 渲染后脏区清空
    tree.render(&mut rec);
    assert!(tree.dirty().is_empty());
}

// ---------- 4. 命令仍然是纯搬运 ----------

#[test]
fn commands_are_routed_not_interpreted() {
    struct Emitter {
        id: String,
        rect: Rect,
    }
    impl Widget for Emitter {
        fn id(&self) -> &str {
            &self.id
        }
        fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
            c.constrain(Size::new(10.0, 10.0))
        }
        fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
            self.rect = r;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, ctx: &mut y901_ui::EventCtx, ev: &UiEvent) -> EventResult {
            match ev {
                UiEvent::Action { name, .. } if name == "ping" => {
                    ctx.emit_app("pong", Json::obj());
                    EventResult::Handled
                }
                _ => EventResult::Ignored,
            }
        }
        fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    }

    let mut tree = UiTree::new(Box::new(Emitter {
        id: "e".into(),
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(50.0, 20.0), &MonoMetrics);
    tree.dispatch_to(
        "e",
        &UiEvent::Action {
            name: "ping".into(),
            args: Json::obj(),
        },
    );
    let cmds = tree.drain_commands();
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::App { name, .. } if name == "pong"));
}

// ---------- 5. RPC ----------

#[test]
fn rpc_get_tree_and_dispatch() {
    let root = Flex::row("root").push(Box::new(Leaf::new("node")));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &MonoMetrics);
    tree.set_focus(Some("node".to_string()));

    let (tx, rx) = channel();
    let (addr, _hub) = y901_agent::rpc::spawn("127.0.0.1:0", tx).expect("bind rpc");

    let mut handle_one = || {
        let pending = rx.recv_timeout(Duration::from_secs(2)).expect("request");
        let result = match &pending.request {
            y901_agent::RpcRequest::GetTree => Ok(tree.snapshot_json()),
            y901_agent::RpcRequest::Query { q } => y901_agent::dispatch_request(
            &mut tree,
            &y901_agent::Policy::default(),
            "query",
            &Json::obj().set("q", q.clone()),
        )
        .map_err(|e| e.code().to_string()),
        y901_agent::RpcRequest::Ping => Ok(Json::obj().set("pong", true)),
            y901_agent::RpcRequest::Focus { node } => {
                if tree.set_focus(Some(node.clone())) {
                    Ok(Json::obj())
                } else {
                    Err(format!("no such node: {node}"))
                }
            }
            y901_agent::RpcRequest::Dispatch { node, .. } => {
                if tree.find(node).is_some() {
                    Ok(Json::obj())
                } else {
                    Err(format!("no such node: {node}"))
                }
            }
            y901_agent::RpcRequest::Invoke { node, .. } => {
                if tree.find(node).is_some() {
                    Ok(Json::obj())
                } else {
                    Err(format!("no such node: {node}"))
                }
            }
            y901_agent::RpcRequest::Subscribe { node, interval_ms } => {
                let ok = tree.subscribe(node, Duration::from_millis(*interval_ms));
                if ok { Ok(Json::obj()) } else { Err(format!("unknown node: {node}")) }
            }
            y901_agent::RpcRequest::Unsubscribe { node } => {
                tree.unsubscribe(node);
                Ok(Json::obj())
            }
            y901_agent::RpcRequest::Unknown { method } => Err(format!("unknown method {method}")),
        };
        let _ = y901_agent::rpc::reply_to(pending, result);
    };

    let client = std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let stream = std::net::TcpStream::connect(addr).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut call = |line: &str| -> Json {
            writeln!(writer, "{line}").unwrap();
            writer.flush().unwrap();
            let mut buf = String::new();
            reader.read_line(&mut buf).unwrap();
            Json::parse(&buf).expect("valid json")
        };
        let a = call(r#"{"id":1,"method":"get_tree"}"#);
        let b = call(r#"{"id":2,"method":"focus","params":{"id":"node"}}"#);
        let c = call(r#"{"id":3,"method":"focus","params":{"id":"ghost"}}"#);
        (a, b, c)
    });

    handle_one();
    handle_one();
    handle_one();

    let (tree_resp, focus_ok, focus_bad) = client.join().unwrap();
    assert_eq!(tree_resp.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        tree_resp
            .get("result")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str()),
        Some("root")
    );
    assert_eq!(focus_ok.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(focus_bad.get("ok").and_then(|v| v.as_bool()), Some(false));
}

// ---------- 6. 兼容性：Flex 直接测量 ----------

#[test]
fn flex_measure_is_cached_too() {
    let mut cache = LayoutCache::new();
    let mut children: Vec<Box<dyn Widget>> = vec![Box::new(Leaf::new("x")), Box::new(Leaf::new("y"))];

    reset_measures();
    let c = Constraints::loose(Size::new(100.0, 100.0));
    let s1 = flex_measure(
        Axis::Vertical,
        y901_ui::Edges::ZERO,
        0.0,
        &mut children,
        &MonoMetrics,
        c,
        &mut cache,
    );
    let first = measure_count();
    let s2 = flex_measure(
        Axis::Vertical,
        y901_ui::Edges::ZERO,
        0.0,
        &mut children,
        &MonoMetrics,
        c,
        &mut cache,
    );
    assert_eq!(first, 2);
    assert_eq!(measure_count(), 2, "第二次应全部命中缓存");
    assert_eq!(s1, s2);
    assert_eq!(Role::PANEL.as_str(), "panel");
}

// ---------- 7. 三阶段路由 + 指针捕获 ----------

type Log = std::rc::Rc<std::cell::RefCell<Vec<(String, EventPhase)>>>;

/// 记录"我在哪个阶段收到了事件"的探针。
struct Probe {
    id: String,
    log: Log,
    capture_pointer: bool,
    rect: Rect,
    item: FlexItem,
}

impl Probe {
    fn new(id: &str, log: Log) -> Self {
        Self {
            id: id.to_string(),
            log,
            capture_pointer: false,
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }
}

impl Widget for Probe {
    fn id(&self) -> &str {
        &self.id
    }
    fn flex(&self) -> FlexItem {
        self.item
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
        c.constrain(Size::new(10.0, 10.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, ctx: &mut y901_ui::EventCtx, event: &UiEvent) -> EventResult {
        self.log.borrow_mut().push((self.id.clone(), ctx.phase));
        match event {
            UiEvent::MouseDown { .. } if self.capture_pointer => {
                ctx.set_pointer_capture(self.id.clone());
                EventResult::Handled
            }
            UiEvent::MouseDown { .. } | UiEvent::MouseUp { .. } | UiEvent::MouseMove { .. } => {
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

/// 单子节点容器：可选在 Capture 阶段把事件停掉。
struct Frame {
    id: String,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    log: Log,
    stop_in_capture: bool,
    handle_in_bubble: bool,
}

impl Frame {
    fn new(id: &str, child: Box<dyn Widget>, log: Log) -> Self {
        Self {
            id: id.to_string(),
            children: vec![child],
            rect: Rect::ZERO,
            log,
            stop_in_capture: false,
            handle_in_bubble: false,
        }
    }
}

impl Widget for Frame {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        cache.measure(self.children[0].as_mut(), m, c)
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = r;
        self.children[0].arrange(r, m, cache);
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn event(&mut self, ctx: &mut y901_ui::EventCtx, ev: &UiEvent) -> EventResult {
        self.log.borrow_mut().push((self.id.clone(), ctx.phase));

        if self.stop_in_capture && ctx.phase == EventPhase::Capture {
            ctx.stop_propagation();
            return EventResult::Handled;
        }
        if self.handle_in_bubble
            && ctx.phase == EventPhase::Bubble
            && matches!(ev, UiEvent::MouseDown { .. })
        {
            return EventResult::Handled;
        }
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

/// 对鼠标事件一律 Ignored 的叶子，用来触发冒泡。
struct Pass {
    id: String,
    rect: Rect,
    log: Log,
}

impl Widget for Pass {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
        c.constrain(Size::new(10.0, 10.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, ctx: &mut y901_ui::EventCtx, _ev: &UiEvent) -> EventResult {
        self.log.borrow_mut().push((self.id.clone(), ctx.phase));
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

fn click(x: f32, y: f32) -> UiEvent {
    UiEvent::MouseDown {
        pos: Pos::new(x, y),
        button: y901_ui::MouseButton::Left,
    }
}

#[test]
fn capture_phase_can_stop_propagation() {
    let log: Log = Default::default();

    let mut root = Frame::new("root", Box::new(Pass { id: "child".into(), rect: Rect::ZERO, log: log.clone() }), log.clone());
    root.stop_in_capture = true;

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(100.0, 100.0), &MonoMetrics);

    tree.dispatch(&click(10.0, 10.0));

    let entries = log.borrow().clone();
    assert_eq!(entries.len(), 1, "只应有根节点在 Capture 收到，实际 {entries:?}");
    assert_eq!(entries[0], ("root".to_string(), EventPhase::Capture));
    assert!(
        !entries.iter().any(|(id, _)| id == "child"),
        "Capture 被 stop 后子节点必须收不到"
    );
}

#[test]
fn three_phases_in_order() {
    let log: Log = Default::default();

    let mut root = Frame::new(
        "parent",
        Box::new(Pass { id: "child".into(), rect: Rect::ZERO, log: log.clone() }),
        log.clone(),
    );
    root.handle_in_bubble = true;

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(100.0, 100.0), &MonoMetrics);

    tree.dispatch(&click(10.0, 10.0));

    let phases = log.borrow().clone();
    assert_eq!(
        phases,
        vec![
            ("parent".to_string(), EventPhase::Capture),
            ("child".to_string(), EventPhase::Target),
            ("parent".to_string(), EventPhase::Bubble),
        ],
        "顺序必须是 Capture → Target → Bubble"
    );
}

#[test]
fn pointer_capture_routes_moves_outside_the_node() {
    let log: Log = Default::default();

    let mut a = Probe::new("a", log.clone()).flex_item(FlexItem::fill());
    a.capture_pointer = true;
    let b = Probe::new("b", log.clone()).flex_item(FlexItem::fill());

    let root = Flex::row("root")
        .push(Box::new(Flex::column("left").flex_item(FlexItem::fill()).push(Box::new(a))))
        .push(Box::new(Flex::column("right").flex_item(FlexItem::fill()).push(Box::new(b))));

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &MonoMetrics);

    // 在左半边按下 → a 捕获指针
    tree.dispatch(&click(20.0, 50.0));
    log.borrow_mut().clear();

    // 移到右半边（几何上命中 b），但仍应派发给 a
    tree.dispatch(&UiEvent::MouseMove { pos: Pos::new(180.0, 50.0) });

    let entries = log.borrow().clone();
    assert!(
        entries.iter().any(|(id, _)| id == "a"),
        "捕获者应收到越界移动，实际 {entries:?}"
    );
    assert!(
        !entries.iter().any(|(id, _)| id == "b"),
        "未被捕获的节点不应收到移动"
    );

    // 抬起后自动释放
    tree.dispatch(&UiEvent::MouseUp {
        pos: Pos::new(180.0, 50.0),
        button: y901_ui::MouseButton::Left,
    });
    log.borrow_mut().clear();

    // 之后的移动回到正常命中测试（b）
    tree.dispatch(&UiEvent::MouseMove { pos: Pos::new(180.0, 50.0) });
    let entries = log.borrow().clone();
    assert!(
        entries.iter().any(|(id, _)| id == "b"),
        "释放后应恢复正常命中，实际 {entries:?}"
    );
}

// ---------- 8. 路径查找 O(d) + Layer 路由 ----------

use std::cell::Cell;
use std::rc::Rc;
use y901_ui::Layer;

/// 每收到一次事件就 +1 的链式容器。
struct Counted {
    id: String,
    kids: Vec<Box<dyn Widget>>,
    rect: Rect,
    count: Rc<Cell<u32>>,
}

impl Widget for Counted {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        match self.kids.first_mut() {
            Some(k) => cache.measure(k.as_mut(), m, c),
            None => c.constrain(Size::new(100.0, 100.0)),
        }
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = r;
        // 只排布首个孩子：多出来的"兄弟"保持零矩形，不参与命中
        if let Some(k) = self.kids.first_mut() {
            k.arrange(r, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn event(&mut self, _ctx: &mut y901_ui::EventCtx, _ev: &UiEvent) -> EventResult {
        self.count.set(self.count.get() + 1);
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

fn chain(depth: usize, count: Rc<Cell<u32>>, with_siblings: usize) -> UiTree {
    // 从叶子往上套 depth 层
    let mut node: Box<dyn Widget> = Box::new(Counted {
        id: format!("n{depth}"),
        kids: Vec::new(),
        rect: Rect::ZERO,
        count: count.clone(),
    });
    for i in (0..depth).rev() {
        node = Box::new(Counted {
            id: format!("n{i}"),
            kids: vec![node],
            rect: Rect::ZERO,
            count: count.clone(),
        });
    }
    // 根节点额外挂一堆兄弟（只为把 n 放大，几何上不参与命中）
    let mut root = Counted {
        id: "root".into(),
        kids: vec![node],
        rect: Rect::ZERO,
        count: count.clone(),
    };
    for i in 0..with_siblings {
        root.kids.push(Box::new(Counted {
            id: format!("s{i}"),
            kids: Vec::new(),
            rect: Rect::ZERO,
            count: count.clone(),
        }));
    }
    // 根必须是 Flex 才能让 kids[0] 拿到整个视口
    UiTree::new(Box::new(Flex::column("frame").push(Box::new(root))))
}

#[test]
fn click_dispatch_is_linear_in_depth() {
    // 32 个计数节点（root + n0..n30）→ Capture 31 + Target 1 + Bubble 31 = 63
    let count = Rc::new(Cell::new(0));
    let mut tree = chain(30, count.clone(), 0);
    tree.layout(Size::new(200.0, 200.0), &MonoMetrics);

    count.set(0);
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 10.0),
        button: y901_ui::MouseButton::Left,
    });
    let small = count.get();
    assert_eq!(small, 63, "32 个节点的链应恰好访问 63 次（2d-1）");

    // 同一深度，但树里多 1000 个节点：访问次数必须完全不变
    let count2 = Rc::new(Cell::new(0));
    let mut big = chain(30, count2.clone(), 1000);
    big.layout(Size::new(200.0, 200.0), &MonoMetrics);

    count2.set(0);
    big.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 10.0),
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(
        count2.get(),
        small,
        "访问次数只与深度有关，与节点总数无关（不是 O(d²) 也不是 O(n·d)）"
    );
}

/// 可精确摆放子节点绝对矩形的容器，用来构造重叠与分层。
struct Stack {
    id: String,
    kids: Vec<Box<dyn Widget>>,
    slots: Vec<Rect>,
    rect: Rect,
}

impl Widget for Stack {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        for k in self.kids.iter_mut() {
            let _ = cache.measure(k.as_mut(), m, c);
        }
        c.constrain(Size::new(200.0, 200.0))
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = r;
        for (k, slot) in self.kids.iter_mut().zip(self.slots.iter()) {
            k.arrange(*slot, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

/// 可以指定 Layer 的叶子。
struct Layered {
    id: String,
    layer: Layer,
    rect: Rect,
    hits: Rc<Cell<u32>>,
    kids: Vec<Box<dyn Widget>>,
}

impl Layered {
    fn new(id: &str, layer: Layer, hits: Rc<Cell<u32>>) -> Self {
        Self {
            id: id.to_string(),
            layer,
            rect: Rect::ZERO,
            hits,
            kids: Vec::new(),
        }
    }
}

impl Widget for Layered {
    fn id(&self) -> &str {
        &self.id
    }

    /// 测试夹具：Modal 内的 `modal_btn` 需要能被 set_focus。
    fn focusable(&self) -> bool {
        true
    }
    fn layer(&self) -> Layer {
        self.layer
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        for k in self.kids.iter_mut() {
            let _ = cache.measure(k.as_mut(), m, c);
        }
        c.constrain(Size::new(100.0, 100.0))
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = r;
        for k in self.kids.iter_mut() {
            k.arrange(r, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn event(&mut self, _ctx: &mut y901_ui::EventCtx, _ev: &UiEvent) -> EventResult {
        self.hits.set(self.hits.get() + 1);
        EventResult::Handled
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
}

fn layered_tree(modal: bool, overlay: bool) -> (UiTree, Rc<Cell<u32>>) {
    let base_hits = Rc::new(Cell::new(0));
    let modal_hits = Rc::new(Cell::new(0));
    let overlay_hits = Rc::new(Cell::new(0));

    let mut kids: Vec<Box<dyn Widget>> = vec![Box::new(Layered::new(
        "base_btn",
        Layer::Base,
        base_hits.clone(),
    ))];
    let mut slots = vec![Rect::new(0.0, 0.0, 200.0, 200.0)];

    if overlay {
        kids.push(Box::new(Layered::new(
            "overlay",
            Layer::Overlay,
            overlay_hits.clone(),
        )));
        slots.push(Rect::new(150.0, 0.0, 50.0, 50.0));
    }
    if modal {
        let mut modal_node = Layered::new("modal", Layer::Modal, modal_hits.clone());
        modal_node.kids.push(Box::new(Layered::new(
            "modal_btn",
            Layer::Base, // 继承父节点的 Modal
            modal_hits.clone(),
        )));
        kids.push(Box::new(modal_node));
        slots.push(Rect::new(50.0, 50.0, 100.0, 100.0));
    }

    let mut tree = UiTree::new(Box::new(Stack {
        id: "root".into(),
        kids,
        slots,
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(200.0, 200.0), &MonoMetrics);
    (tree, base_hits)
}

#[test]
fn modal_blocks_clicks_outside_and_traps_focus() {
    let (mut tree, base_hits) = layered_tree(true, false);
    let modal_hits = Rc::new(Cell::new(0));
    let _ = &modal_hits;

    // 点 Modal 外部（左上角）：事件被吞掉，底层收不到
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 10.0),
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(base_hits.get(), 0, "Modal 外部点击不能穿透到底层");
    assert!(tree.hit_path(Pos::new(10.0, 10.0)).is_none(), "命中应被吞掉");

    // 点 Modal 内部：命中 Modol 子树
    let hit = tree.hit_path(Pos::new(60.0, 60.0)).expect("应命中 Modal 内部");
    let id = tree.hit_test(Pos::new(60.0, 60.0)).unwrap();
    assert!(id == "modal_btn" || id == "modal", "实际命中 {id}");

    // 焦点被限制在 Modal 内
    assert!(!tree.set_focus(Some("base_btn".into())), "Modal 外节点不能获焦");
    assert!(tree.set_focus(Some("modal_btn".into())), "Modal 内节点可以获焦");
    assert_eq!(tree.focus(), Some("modal_btn"));
    let _ = hit;
}

#[test]
fn overlay_is_on_top_but_lets_clicks_through() {
    let (mut tree, base_hits) = layered_tree(false, true);

    // Overlay 覆盖 (150,0)-(200,50)：点这里应命中 Overlay 而不是底层
    let id = tree.hit_test(Pos::new(160.0, 10.0)).unwrap();
    assert_eq!(id, "overlay");

    // Overlay 之外：点击正常落到 Base（能"穿透"）
    let id = tree.hit_test(Pos::new(10.0, 10.0)).unwrap();
    assert_eq!(id, "base_btn");

    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(10.0, 10.0),
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(base_hits.get(), 1, "Overlay 不应阻挡底层点击");
}

#[test]
fn layers_paint_in_order() {
    let (tree, _) = layered_tree(true, true);
    assert!(tree.has_layer(Layer::Overlay));
    assert!(tree.has_layer(Layer::Modal));
    // Modal 根路径必须存在，且是作用域
    assert!(tree.modal_scope().is_some());
}

// ---------- 9. 广播不阻塞 UI ----------

#[test]
fn broadcast_never_blocks_ui() {
    use std::time::{Duration, Instant};

    // 真实监听 + 一个"只连不读"的客户端：写线程会卡在 socket 上，
    // 队列填满后 `send` 必须丢包而不是阻塞。
    let (tx, _rx) = std::sync::mpsc::channel();
    let (addr, hub) = y901_agent::rpc::spawn("127.0.0.1:0", tx).expect("bind");
    let _client = std::net::TcpStream::connect(addr).expect("connect");
    std::thread::sleep(Duration::from_millis(50)); // 等连接注册

    // 每条 4KB：几百条就能把内核缓冲 + 1024 深度队列灌满
    let payload = format!("{{\"event\":\"tick\",\"blob\":\"{}\"}}", "x".repeat(4000));
    let start = Instant::now();
    for _ in 0..3000 {
        hub.send(&payload);
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "3000 次投递必须立即返回（不阻塞 UI），实际 {elapsed:?}"
    );
    assert!(
        hub.dropped() > 0,
        "队列应被灌满并丢弃多余推送，实际 dropped={}",
        hub.dropped()
    );
}

#[test]
fn broadcast_writer_removes_dead_connections() {
    // 客户端连上后立刻关闭：写线程下一次写失败 → 线程退出 → 连接被注销。
    let (tx, _rx) = std::sync::mpsc::channel();
    let (addr, hub) = y901_agent::rpc::spawn("127.0.0.1:0", tx).expect("bind");
    {
        let stream = std::net::TcpStream::connect(addr).expect("connect");
        drop(stream);
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut count = usize::MAX;
    while std::time::Instant::now() < deadline {
        hub.send("{\"event\":\"ping\"}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        count = hub.connection_count();
        if count == 0 {
            break;
        }
    }
    assert_eq!(count, 0, "断开连接必须被注销");
}