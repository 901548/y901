//! 收尾：RPC 每请求鉴权 + ScrollView 视口外不可命中（走真实 dispatch）。

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::channel;
use std::time::Duration;

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Pos, Rect, RenderCtx, Size, TextStyle,
    TextMetrics, UiEvent, UiTree, Widget,
};

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

// ---------- 1. RPC 鉴权 ----------

#[test]
fn rpc_rejects_requests_without_token() {
    // 显式传 token，不依赖进程级 env（否则与另一个测试并行时互相干扰）
    let (tx, _rx) = channel();
    let (addr, _hub) = y901_agent::rpc::spawn_with_token("127.0.0.1:0", tx, "t0ken".into())
        .expect("bind");

    let stream = std::net::TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    // 无 token
    writeln!(writer, r#"{{"id":1,"method":"get_tree"}}"#).unwrap();
    writer.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("unauthorized"), "无 Token 应被拒：{line}");
    assert!(line.contains("401"), "应返回 401：{line}");

    // 连接应被断开：后续读返回 EOF
    let mut rest = String::new();
    let n = reader.read_line(&mut rest).unwrap_or(0);
    assert_eq!(n, 0, "鉴权失败后应断开连接");
}

#[test]
fn rpc_accepts_requests_with_token() {
    let (tx, rx) = channel();
    let (addr, _hub) = y901_agent::rpc::spawn_with_token("127.0.0.1:0", tx, "t0ken2".into())
        .expect("bind");

    let stream = std::net::TcpStream::connect(addr).expect("connect");
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writeln!(writer, r#"{{"id":1,"method":"ping","params":{{"token":"t0ken2"}}}}"#).unwrap();
    writer.flush().unwrap();

    // UI 侧处理
    let pending = rx.recv_timeout(Duration::from_secs(10)).expect("request");
    let _ = y901_agent::rpc::reply_to(pending, Ok(y901_ui::Json::obj().set("pong", true)));

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("pong"), "带正确 Token 应放行：{line}");

    std::env::remove_var("Y901_TOKEN");
}

// ---------- 2. ScrollView 视口外不可命中 ----------

struct Tracked {
    id: String,
    rect: Rect,
    clicks: std::rc::Rc<std::cell::RefCell<usize>>,
}

impl Widget for Tracked {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(Size::new(c.max.w, 40.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::MouseDown { .. } => {
                *self.clicks.borrow_mut() += 1;
                EventResult::Handled
            }
            // 其余事件放行，让滚轮冒泡到 ScrollView
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default().exposed(true)
    }
}

fn spacer(id: &str) -> Box<dyn Widget> {
    Box::new(Tracked {
        id: id.to_string(),
        rect: Rect::ZERO,
        clicks: std::rc::Rc::new(std::cell::RefCell::new(0)),
    })
}

#[test]
fn scrollview_child_outside_viewport_is_not_clickable_until_scrolled() {
    let clicks = std::rc::Rc::new(std::cell::RefCell::new(0usize));

    // 内容 400px：5 个 40px 行 + 目标按钮在最后
    let mut column = Flex::column("content");
    for i in 0..4 {
        column = column.push(spacer(&format!("pad{i}")));
    }
    column = column.push(Box::new(Tracked {
        id: "target".into(),
        rect: Rect::ZERO,
        clicks: clicks.clone(),
    }));

    let sv = y901_widgets::ScrollView::new("sv", Box::new(column))
        .flex_item(FlexItem::fill());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(sv))));
    // 视口 200 高，内容 200（4×40 + 40）
    tree.layout(Size::new(300.0, 200.0), &Mono);

    // 目标在 y=160..200，仍在视口内 —— 先把它推出视口
    // 再加高内容：直接把视口压到 120
    tree.layout(Size::new(300.0, 120.0), &Mono);

    let screen = Pos::new(50.0, 180.0); // 目标顶部 160 之后的位置，已在视口(0..120)外

    // 真实 dispatch：视口外的点击不应到达 target
    tree.dispatch(&UiEvent::MouseDown {
        pos: screen,
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(*clicks.borrow(), 0, "视口外的子节点不应收到点击");

    // 滚动使 target 进入视口
    tree.dispatch(&UiEvent::Wheel {
        pos: Pos::new(50.0, 60.0),
        delta: -480.0,
    });
    tree.layout(Size::new(300.0, 120.0), &Mono);

    let target = tree.rect_of("target").expect("target");
    let center = Pos::new(target.x + 20.0, target.y + 20.0);
    assert!(
        target.y >= 0.0 && target.y < 120.0,
        "滚动后目标应进入视口：{target:?}"
    );

    tree.dispatch(&UiEvent::MouseDown {
        pos: center,
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(*clicks.borrow(), 1, "进入视口后应能点中");
}
