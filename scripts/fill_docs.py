#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""按 cargo 的 missing_docs 输出，给指定公开项插入 `///` 文档注释。

设计要点：
- 以"文件 + 行号 + 该行代码片段"三元组定位，插入前校验片段一致，避免行号漂移插错位置。
- 同一文件内**从下往上**插入，避免前面的插入影响后面的行号。
- 已经紧邻 `///` 的项跳过（幂等，可重复运行）。
"""

import io
import re
import subprocess
import sys

ROOT = "F:/Y901"

# 片段 -> 文档（片段取自 cargo 警告里打印的源码行）
DOCS = {
    # ---- y901-extras/palette.rs ----
    "pub fg: ColorSpec,": "默认前景色。`SGR 39` 恢复到它。",
    "pub bg: ColorSpec,": "默认背景色。`SGR 49` 恢复到它。",
    "pub bold: bool,": "粗体标志（`SGR 1` / `22`）。",
    "pub underline: bool,": "下划线标志（`SGR 4` / `24`）。",
    "pub reverse: bool,": "反色标志（`SGR 7` / `27`）。与选区反色共用同一次交换，两者叠加会翻回原色。",
    # ---- y901-extras/vt.rs ----
    "pub struct VtFilter {": (
        "增量 VT 解析器：把 PTY 字节流翻译成网格操作。\n"
        "为什么是增量的：转义序列可能被拆在两次 read 之间，状态必须跨调用保留。"
    ),
    # ---- y901-render/atlas.rs：Slot（字形在图集里的占位）----
    "pub x: u32,": "矩形左上角 X（图集像素）。",
    "pub y: u32,": "矩形左上角 Y（图集像素）。",
    "pub w: u32,": "矩形宽度（像素）。",
    "pub h: u32,": "矩形高度（像素）。",
    "pub last_used: u64,": "最近一次使用的帧号；LRU 驱逐按它排序。",
    "pub fn padded_w(&self) -> u32 {": "含 1px 间隙的宽度。间隙是为了防止相邻字形采样时互相渗色。",
    "pub fn padded_h(&self) -> u32 {": "含 1px 间隙的高度。",
    "pub fn area(&self) -> u32 {": "含间隙面积；打包器用它比较候选位置。",
    # ---- y901-render/atlas.rs：SkylinePacker ----
    "pub fn new(size: u32) -> Self {": "建一个边长为 `size` 的空图集打包器（初始只有一个整宽平台）。",
    "pub fn reset(&mut self) {": "清空所有平台并恢复到初始状态（整张图集重新可用）。",
    # ---- y901-render/atlas.rs：AtlasRect ----
    "pub fn size(&self) -> u32 {": "图集边长（像素）。",
    "pub fn data(&self) -> &[u8] {": "图集像素（RGBA；`rgb` 存覆盖率，`a` 恒为 255）。",
    "pub fn frame(&self) -> u64 {": "当前帧号；每 `begin_frame` 递增，供 LRU 使用。",
    "pub fn slot(&self, ch: char) -> Option<&Slot> {": "查某个字符的占位（没打包过返回 `None`）。",
    "pub fn slot_count(&self) -> usize {": "已打包的字符数（诊断与内存估算用）。",
    "pub fn compact_count(&self) -> u64 {": "发生过的整图重建次数。频繁重建说明图集太小或 LRU 太激进。",
    # ---- y901-mcp/bridge.rs ----
    "pub line: String,": "一条 JSON-RPC 原始行（行分隔协议，不做预解析）。",
    "pub reply: Sender<Option<String>>,": "回传通道；`Some` 是响应行，`None` 表示本次请求不需要回包。",
    # ---- y901-widgets ----
    "pub struct Button {": "可点击按钮。点击不直接改状态，而是发一条应用命令——组件互不认识。",
    "pub struct TextLabel {": "静态文本；挂上 `Signal` 后由数据源驱动更新。",
    "pub struct VerticalList {": "垂直列表容器：等宽、依次排列、支持 gap 与 padding。",
    "pub struct Menu {": "下拉/右键菜单。作为浮层内容时需先调 `open()`——`render`/`event` 都以它为门。",
    "pub fn set_open(&mut self, open: bool) {": "直接设置开启状态（不重置高亮）；用于由宿主控制弹层生命周期。",
    "pub struct SplitView {": "两栏分隔容器：可拖动分隔条，两侧各有最小尺寸。",
    "pub struct TextInput {": "单行文本输入框：光标、选区、滚动、剪贴板、IME。",
    "fn get(&mut self) -> Option<String>;": "读取剪贴板文本；`None` 表示不可用或为空（控件只需安静地不粘）。",
    "fn set(&mut self, text: &str);": "写入剪贴板；失败被忽略——剪贴板不可用不应阻断 UI。",
    "pub italic: bool,": "斜体标志。",
    "pub color: Option<Color>,": "覆盖颜色；`None` = 用主题色。",
    "pub text: String,": "本段文本。",
    "pub style: InlineStyle,": "本段样式（与文本一起构成 `Span`）。",
    "pub lang: String,": "代码块语言名（仅用于标识，不做高亮）。",
    "lines: Vec<String>,": "预切好的行；避免每帧重新切分。",
    # ---- y901-render/font.rs ----
    "pub struct DirtyRect {": "图集脏矩形：告诉后端这一块像素变了，需要重新上传。",
    "pub struct GlyphInfo {": (
        "一个已光栅化字形的度量与图集位置。\n"
        "为什么带 `bearing`：字形墨迹不一定从原点开始，绘制时要按 bearing 偏移，\n"
        "否则字符间距会忽大忽小。"
    ),
    "pub width: u32,": "墨迹宽度（像素）。",
    "pub height: u32,": "墨迹高度（像素）。",
    "pub bearing_y: i32,": "墨迹相对基线的垂直偏移（向上为正）。",
    "pub advance: f32,": "前进量（像素）；等宽网格里用于居中补偿。",
    "pub atlas_y: u32,": "图集内 Y 坐标。",
    "pub fn is_empty(&self) -> bool {": "是否为空字形（空格等无墨迹字符）。空字形不该写图集，也不该画。",
    "pub fn cell_metrics_for(font_size: f32) -> (f32, f32) {": (
        "由字号推出等宽单元格尺寸 `(宽, 高)`。\n"
        "为什么要独立函数：布局、字形引擎、后端必须用同一套格尺寸，否则文字与光标会错位。"
    ),
}

KIND_HINT = re.compile(r"missing documentation for (.+)$")
LOC = re.compile(r"--> (crates[\\/][^:]+):(\d+)")


def collect() -> list[tuple[str, int, str]]:
    out = subprocess.run(
        ["cargo", "build", "--workspace", "--offline"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    ).stderr
    items = []
    pending = None
    for line in out.splitlines():
        m = KIND_HINT.search(line)
        if m:
            pending = m.group(1)
            continue
        m = LOC.search(line)
        if m and pending:
            items.append((m.group(1), int(m.group(2)), pending))
            pending = None
    return items


def snippet_at(path: str, lineno: int) -> str:
    with io.open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    return lines[lineno - 1].strip() if 0 < lineno <= len(lines) else ""


def patch(path: str, lineno: int) -> str | None:
    """在 lineno 行之上插入文档；返回插入的文档（`None` 表示跳过）。"""
    with io.open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    idx = lineno - 1
    if idx >= len(lines):
        return None
    if idx > 0 and lines[idx - 1].strip().startswith("///"):
        return None  # 已有文档
    snip = lines[idx].strip()
    doc = DOCS.get(snip)
    if doc is None:
        return None
    indent = lines[idx][: len(lines[idx]) - len(lines[idx].lstrip())]
    block = [f"{indent}/// {part}" if part else f"{indent}///" for part in doc.split("\n")]
    lines[idx:idx] = block
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines))
    return doc


def main() -> int:
    items = collect()
    if not items:
        print("没有 missing_docs 警告")
        return 0
    # 每个文件从下往上插入
    items.sort(key=lambda t: (t[0], -t[1]))
    done = skipped = 0
    for path, lineno, kind in items:
        full = f"{ROOT}/{path.replace(chr(92), '/')}"
        snip = snippet_at(full, lineno)
        if patch(full, lineno) is None:
            skipped += 1
            print(f"[skip] {path}:{lineno} ({kind}) :: {snip[:70]}")
        else:
            done += 1
    print(f"插入 {done} 处，跳过 {skipped} 处（未在 DOCS 表中或已有文档）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
