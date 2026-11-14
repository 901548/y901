//! 字体加载：`fetch` + `FontFace`，就绪前出占位帧（`docs/WASM.md` §3.5）。

/// 加载字体。**异步签名待定**（S3）。
pub fn load_font(_url: &str) {
    todo!("S3：fetch + FontFace，就绪后全量失效重绘")
}