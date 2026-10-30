//! 端到端：真实二进制 `y901-app --mcp --headless` 通过 stdin/stdout 驱动真实 UiTree。

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn run_session(lines: &[&str]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_y901"))
        .arg("--mcp")
        .arg("--headless")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn y901-app");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for line in lines {
            writeln!(stdin, "{line}").expect("write");
        }
        stdin.flush().ok();
    }
    // 关闭 stdin，让服务端读完退出
    drop(child.stdin.take());

    let stdout = child.stdout.take().expect("stdout");
    let lines: Vec<String> = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .collect();
    let _ = child.wait();
    lines
}

#[test]
fn app_mcp_mode_serves_handshake_and_tool_calls() {
    let out = run_session(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ]);

    assert_eq!(out.len(), 2, "通知不占位，应有 2 条响应：{out:?}");

    let first = &out[0];
    assert!(first.contains("\"id\":1"));
    assert!(first.contains("protocolVersion"));
    assert!(first.contains("\"name\":\"y901\""));

    let second = &out[1];
    assert!(second.contains("\"id\":2"));
    for tool in ["ui_click", "ui_type_text", "ui_scroll", "ui_invoke"] {
        assert!(second.contains(tool), "tools/list 应包含 {tool}");
    }
}

#[test]
fn app_mcp_mode_dispatches_click_into_real_tree() {
    // 点最小启动树里的标签节点：验证 dispatch 链路可达
    let out = run_session(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"ui://tree"}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"title"}}}"#,
    ]);

    assert_eq!(out.len(), 3, "应有 3 条响应：{out:?}");

    // 1) 真实树里能看到终端节点
    let tree = &out[1];
    assert!(
        tree.contains("title"),
        "最小启动树应包含 title 标签：{tree}"
    );

    // 2) 点击被真实 UiTree 处理
    let click = &out[2];
    assert!(click.contains("\"isError\":false"), "点击不应报错：{click}");
    assert!(
        click.contains("handled") || click.contains("\"ok\":true"),
        "点击应被派发：{click}"
    );
}

#[test]
fn app_mcp_mode_rejects_calls_before_initialize() {
    let out = run_session(&[
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"demo2"}}}"#,
    ]);
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("-32002"), "未握手应拒绝：{}", out[0]);
}
