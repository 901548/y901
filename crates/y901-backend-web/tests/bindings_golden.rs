//! **手写 DOM 绑定的 golden 断言**。
//!
//! 目的：`#[wasm_bindgen]` 的 `js_name` / `js_class` / `method` / `getter` / `setter` /
//! `constructor` 只在 **wasm32** 编译时才生成 JS 胶水；写错了 native 编译照样通过，
//! 要到浏览器里才炸（历史上 `window is not a function`、`getContext 1 argument required`
//! 都是这么来的）。
//!
//! 做法：把 `bindings.rs` 的**源码文本**当快照来断言。这不是"运行期验证"，
//! 而是"**声明表面**不能被悄悄改动"：任何人改了一个 `js_name`，这里立刻红，
//! 必须显式改测试才能继续——于是改动变成有意识的行为，而不是手滑。
//!
//! 为什么不在 native 上真的调用：这些类型只在 wasm32 存在，native 下连 `extern "C"`
//! 都不会链接 JS。源码快照是唯一能在 CI（native）里跑的形式。

const SRC: &str = include_str!("../src/bindings.rs");

/// 收集源码里所有 `js_name = "…"` 的值（引号形式）。
fn quoted_js_names() -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = SRC;
    loop {
        let Some(pos) = rest.find("js_name = \"") else {
            break;
        };
        let after = &rest[pos + "js_name = \"".len()..];
        let Some(end) = after.find('"') else {
            break;
        };
        out.push(after[..end].to_string());
        rest = &after[end..];
    }
    out
}

/// 收集所有 `js_namespace = …` 的命名空间名。
fn js_namespaces() -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = SRC;
    loop {
        let Some(pos) = rest.find("js_namespace = ") else {
            break;
        };
        let after = &rest[pos + "js_namespace = ".len()..];
        let end = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        out.push(after[..end].to_string());
        rest = &after[end..];
    }
    out
}

/// **全部引号形式的 `js_name` 值**（排序去重）。
///
/// 改 `bindings.rs` 时若增减或重命名，这里的期望集合要**手工**跟着改——
/// 这正是本测试的意图：让改动必须被看见。
const EXPECTED_JS_NAMES: &[&str] = &[
    "CSSStyleDeclaration",
    "CanvasRenderingContext2D",
    "CompositionEvent",
    "Document",
    "Element",
    "Event",
    "EventTarget",
    "HTMLInputElement",
    "HtmlCanvasElement",
    "ImageData",
    "KeyboardEvent",
    "MouseEvent",
    "Uint8ClampedArray",
    "WheelEvent",
    "activeElement",
    "addEventListener",
    "altKey",
    "appendChild",
    "cancelAnimationFrame",
    "clientHeight",
    "clientWidth",
    "clientX",
    "clientY",
    "console.log",
    "createElement",
    "createImageData",
    "ctrlKey",
    "deltaY",
    "getAttribute",
    "getContext",
    "getElementById",
    "getPropertyPriority",
    "getPropertyValue",
    "isComposing",
    "keyCode",
    "metaKey",
    "offsetX",
    "offsetY",
    "preventDefault",
    "putImageData",
    "remove",
    "requestAnimationFrame",
    "set",
    "setAttribute",
    "setProperty",
    "shiftKey",
    "style",
    "tagName",
    "window",
];

#[test]
fn every_quoted_js_name_is_accounted_for() {
    let mut actual = quoted_js_names();
    actual.sort();
    actual.dedup();
    assert_eq!(
        actual,
        EXPECTED_JS_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        "bindings.rs 里的 js_name 集合变了。若是有意改动，请同步更新 EXPECTED_JS_NAMES（本次 diff 就是审阅点）"
    );
}

#[test]
fn namespaced_bindings_are_pinned() {
    // Date.now() 与 console.log 是两个特殊形态：前者是命名空间静态方法，
    // 后者 js_name 含点号。写错都是"静默失败到运行时"。断言它们以**原文**出现。
    assert!(
        SRC.contains("#[wasm_bindgen(js_namespace = Date, js_name = now)]"),
        "Date.now 的绑定形态变了"
    );
    assert!(
        SRC.contains("#[wasm_bindgen(js_name = \"console.log\")]"),
        "console.log 的绑定形态变了"
    );
    assert_eq!(js_namespaces(), vec!["Date".to_string()], "命名空间集合变了");
}

/// getter / setter / constructor / indexing_setter 的数量是"表面宽度"的度量：
/// 任何一个被误删（比如把 getter 写成普通 method）都会改变这些计数。
///
/// **只统计 `#[wasm_bindgen…]` 属性行**，避免文档注释里的同名词把数字弄脏。
#[test]
fn binding_attribute_counts_are_stable() {
    fn count(pat: &str) -> usize {
        SRC.lines()
            .filter(|l| l.trim_start().starts_with("#[wasm_bindgen"))
            .filter(|l| l.contains(pat))
            .count()
    }
    assert_eq!(count("method, getter"), 32, "getter 数量变了");
    assert_eq!(count("method, setter"), 3, "setter 数量变了");
    assert_eq!(count("constructor"), 2, "constructor 数量变了");
    // 注意：`JsObject::set_prop` 与 `Uint8ClampedArray::set_index` 已删除，
    // 所以 indexing_setter 现在为 0（若未来重新引入，这里会提醒你更新）。
    assert_eq!(count("indexing_setter"), 0, "indexing_setter 数量变了");
    assert_eq!(count("structural"), 4, "structural 数量变了");
    // setProperty 有两个重载（带 priority / 不带），都要保留。
    assert_eq!(count("js_name = \"setProperty\""), 2, "setProperty 重载少了");
}

/// 关键类型名不能被改：它们与 JS 侧构造函数一一对应。
#[test]
fn dom_type_names_are_pinned() {
    for name in [
        // 注意：类型名与 JS 名不一定相同——`Window` 对应 JS 小写的 `window`。
        "window",
        "HtmlCanvasElement",
        // JS 层类名是大写的 `HTMLInputElement`，与 Rust 类型名不同。
        "HTMLInputElement",
        "CanvasRenderingContext2D",
        "ImageData",
        "Uint8ClampedArray",
        "KeyboardEvent",
        "MouseEvent",
        "WheelEvent",
        "CompositionEvent",
        "EventTarget",
        "Element",
        "Document",
    ] {
        assert!(
            SRC.contains(&format!("js_name = \"{name}\"")),
            "类型 {name} 的 js_name 不见了或被改名"
        );
    }
}

/// `global_window` 必须是 **thread_local_v2 静态**（不是函数）：
/// 早先写成 `fn global_window` 导致浏览器报 `window is not a function`。
#[test]
fn global_window_stays_a_thread_local_static() {
    assert!(
        SRC.contains("#[wasm_bindgen(thread_local_v2, js_name = \"window\")]"),
        "global_window 的声明形态变了（必须是 thread_local_v2 静态，否则运行时 TypeError）"
    );
    assert!(
        SRC.contains("static GLOBAL_WINDOW: Window;"),
        "GLOBAL_WINDOW 静态不见了"
    );
}
