//! MCP 端到端冒烟测试：内存 stdin/stdout 走完整双线程桥 + 握手状态机 + 脱敏。

use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use y901_agent::{McpServer, Policy};
use y901_ui::{Flex, FlexItem, Semantics, Size, TextMetrics, TextStyle, UiTree, Widget};

struct Mono;
impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.len() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

struct Probe {
    id: String,
    exposed: bool,
    sensitive: bool,
    rect: y901_ui::Rect,
}

impl Widget for Probe {
    fn id(&self) -> &str {
        &self.id
    }
    fn flex(&self) -> FlexItem {
        FlexItem::fill()
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: y901_ui::Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(120.0, 32.0))
    }
    fn arrange(
        &mut self,
        r: y901_ui::Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> y901_ui::Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut y901_ui::EventCtx, _e: &y901_ui::UiEvent) -> y901_ui::EventResult {
        y901_ui::EventResult::Handled
    }
    fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(&self.id)
            .value(if self.sensitive { "s3cret" } else { "public" })
            .action("go")
            .state("token", if self.sensitive { "abc123" } else { "none" })
            .exposed(self.exposed)
            .sensitive(self.sensitive)
    }
}

fn tree() -> UiTree {
    let root = Flex::column("root")
        .push(Box::new(Probe {
            id: "ok".into(),
            exposed: true,
            sensitive: false,
            rect: y901_ui::Rect::ZERO,
        }))
        .push(Box::new(Probe {
            id: "vault".into(),
            exposed: true,
            sensitive: true,
            rect: y901_ui::Rect::ZERO,
        }))
        .push(Box::new(Probe {
            id: "hidden".into(),
            exposed: false,
            sensitive: false,
            rect: y901_ui::Rect::ZERO,
        }));
    let mut t = UiTree::new(Box::new(root));
    t.layout(Size::new(400.0, 300.0), &Mono);
    t
}

/// 把若干行喂进桥，返回 stdout 的所有行。
fn run(lines: &[&str]) -> Vec<String> {
    let input = Cursor::new(lines.join("\n").into_bytes());
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let jobs = y901_mcp::bridge::spawn_io(input, SharedWriter(output.clone()));

    let mut tree = tree();
    let mut server = McpServer::new(Policy::default());
    // 阻塞泵到输入结束（Cursor 读完即 drop 掉 job_tx）
    y901_mcp::bridge::run_blocking(&mut server, &mut tree, &jobs);

    // 给 stdout 线程一点时间把行写完
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(2));
        if !output.lock().unwrap().is_empty() {
            std::thread::sleep(Duration::from_millis(20));
            break;
        }
    }

    let bytes = output.lock().unwrap().clone();
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(|l| l.to_string())
        .collect()
}

#[test]
fn stdio_bridge_serves_initialize_list_and_call_in_order() {
    let out = run(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"ok"}}}"#,
    ]);

    assert!(out.len() >= 3, "应至少输出 3 行响应，实际 {out:?}");

    let init: serde_free::Json = serde_free::Json::parse(&out[0]).expect("json");
    assert_eq!(init.id(), 1.0);
    assert!(init.raw().contains("protocolVersion"));

    let list = serde_free::Json::parse(&out[1]).expect("json");
    assert_eq!(list.id(), 3.0, "通知不占位，第二条响应应属于 id=3");
    assert!(list.raw().contains("ui_click"));

    let call = serde_free::Json::parse(&out[2]).expect("json");
    assert_eq!(call.id(), 4.0);
    assert!(call.raw().contains("\"isError\":false"));
}

#[test]
fn uninitialized_session_rejects_tools_call() {
    let out = run(&[
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"ok"}}}"#,
    ]);

    assert!(!out.is_empty(), "应有错误响应，实际 {out:?}");
    let body = out.join("\n");
    assert!(body.contains("-32002"), "未初始化应返回 -32002：{body}");
    assert!(body.contains("not initialized"), "错误消息应可读：{body}");
}

#[test]
fn node_resource_redacts_state_fields() {
    let out = run(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"ui://node/vault"}}"#,
    ]);

    let body = out.join("\n");
    assert!(!body.is_empty(), "应有响应");
    assert!(!body.contains("s3cret"), "value 必须脱敏：{body}");
    assert!(
        !body.contains("abc123"),
        "state 里的敏感值也必须脱敏：{body}"
    );
    assert!(body.contains("***"));
}

/// 极简 JSON 读取器（只为本测试取 id / 原文，不进生产代码）。
mod serde_free {
    pub struct Json(String);
    impl Json {
        pub fn parse(s: &str) -> Option<Self> {
            if s.trim().starts_with('{') {
                Some(Json(s.to_string()))
            } else {
                None
            }
        }
        pub fn id(&self) -> f64 {
            let key = "\"id\":";
            let i = self.0.find(key).expect("id") + key.len();
            self.0[i..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0.0)
        }
        pub fn raw(&self) -> &str {
            &self.0
        }
    }
}
