//! 性能与裁剪回归：主循环空闲必须休眠；客户区必须有基座裁剪。
//!
//! 本机无 GPU，无法实测帧时间，所以用结构断言守住两个修复点，
//! 并用 `clip_quad` 的单元测试证明"越界字形会被裁掉"。

/// 主循环在没有事件、没有脏区时必须阻塞等待（否则 100% CPU 空转）。
#[test]
fn runtime_idles_when_nothing_to_do() {
    let src = include_str!("../src/runtime.rs");
    let idle = src
        .find("wait_message(")
        .expect("主循环必须调用 wait_message 休眠");
    let render = src.find("window.render(&mut tree)").expect("必须有 render");
    assert!(idle > render, "休眠必须发生在 render 之后（否则会漏帧）");
    assert!(
        src.contains("!had_work && tree.dirty().is_empty()"),
        "只有「没有事件且没有脏区」时才允许休眠（有 PTY 数据时不能睡）"
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
    assert!(
        body.contains("self.clips.push(Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h))"),
        "begin() 必须压入客户区裁剪（否则内容会画到窗口外）：\n{body}"
    );
    assert!(
        body.contains("self.clips.clear()"),
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
