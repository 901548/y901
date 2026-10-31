//! 结构断言：终端渲染路径不得有逐字符 `to_string()`。
//!
//! 为什么不用分配计数器：在测试里替换全局分配器（`#[global_allocator]`）会污染
//! 整个测试二进制，且 `String` 分配可能来自框架其他层，难以归因到"这一格"。
//! 源码级断言更直接：只要 `render` 方法体里没有 `to_string`，就不会每格分配。

#[test]
fn terminal_render_has_no_to_string() {
    let src = include_str!("../src/terminal.rs");

    // 切出 render 方法体：从 "fn render(" 到下一个顶层方法（4 空格缩进 + "fn "）
    let start = src.find("fn render(").expect("terminal::render 存在");
    let rest = &src[start..];
    let end = rest[1..]
        .find("\n    fn ")
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    let body = &rest[..end];

    assert!(
        !body.contains("to_string()"),
        "渲染路径不得逐字符分配，body:\n{body}"
    );
    assert!(
        body.contains("draw_char"),
        "每格单字符必须走 draw_char（零堆分配），body:\n{body}"
    );
}
