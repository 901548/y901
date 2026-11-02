#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""第 2 轮文档补齐：y901-core + y901-backend-win32（83 项）。

复用 `fill_docs.py` 的定位与插入逻辑，只是把文档表扩成"文件::片段"级别的精确映射
（同一个片段在不同文件里含义不同，例如 `Hresult` 在 d3d11.rs 与 font.rs 都出现）。
"""

import io
import sys

sys.path.insert(0, "F:/Y901/scripts")
import fill_docs  # noqa: E402

EXTRA = {
    # ---------------- y901-core ----------------
    "lib.rs::pub const STYLE_DEFAULT: u16 = 0;": (
        "默认样式的固定下标。为什么要有它：空格/空白单元格占绝大多数，"
        "如果每次都去 `intern` 一遍，样式表会在启动时就被撑大。"
    ),
    "lib.rs::Default,": "使用宿主默认色。具体是什么颜色由渲染后端/主题决定——数据层不认识主题。",
    "lib.rs::Rgb(u8, u8, u8),": "24 位真彩色。",
    "lib.rs::pub const fn is_default(self) -> bool {": (
        "是否是\"未指定颜色\"。调用方据此决定要不要去查主题，"
        "而不是把默认色提前解析成具体 RGB（那样换主题就不会生效）。"
    ),
    "lib.rs::pub fg: ColorSpec,": "前景色；`Default` 表示用主题前景。",
    "lib.rs::pub bg: ColorSpec,": "背景色；`Default` 表示透明/继承下层。",
    "lib.rs::pub bold: bool,": "粗体。",
    "lib.rs::pub underline: bool,": "下划线。",
    "lib.rs::pub reverse: bool,": "反色：渲染时交换前景与背景。",
    "lib.rs::pub const fn fg(mut self, color: ColorSpec) -> Self {": "设置前景色并返回自身（链式构造，便于写常量样式）。",
    "lib.rs::pub const fn bg(mut self, color: ColorSpec) -> Self {": "设置背景色并返回自身。",
    "lib.rs::pub fn is_plain(&self) -> bool {": (
        "是否等价于默认样式。渲染路径靠它跳过样式解析——绝大多数单元格都是纯默认样式。"
    ),
    "lib.rs::pub fn intern(&mut self, style: CellStyle) -> u16 {": (
        "把样式去重后返回下标。为什么用下标而不是内联结构体：单元格只需 2 字节，"
        "1 万个单元格不必各自存一份完整样式。"
    ),
    "lib.rs::pub fn get(&self, id: u16) -> CellStyle {": (
        "按下标取回样式。未知下标返回默认样式而不是 panic——渲染路径不该因为数据问题崩掉。"
    ),
    "lib.rs::pub fn len(&self) -> usize {": "已收录的样式数（去重后的数量，可用于评估样式表规模）。",
    "lib.rs::pub fn is_empty(&self) -> bool {": "是否还没有任何样式。",
    "lib.rs::pub enum CellKind {": "单元格的宽度种类：东亚宽字符占两格，必须成对标记，否则光标与选中会错位。",
    "lib.rs::Narrow = 0,": "窄字符（占 1 格）。",
    "lib.rs::WideHead = 1,": "宽字符的左半（后一格是 `WideTail`）。",
    "lib.rs::WideTail = 2,": "宽字符的右半：渲染时跳过，避免把同一个字形画两遍。",
    "lib.rs::pub ch: char,": "字符内容；`WideTail` 的 `ch` 无意义。",
    "lib.rs::pub style: u16,": "样式表下标（见 `StyleTable::intern`）。",
    "lib.rs::pub kind: CellKind,": "宽度种类。",
    "lib.rs::pub const fn blank(style: u16) -> Self {": "空白单元格（空格，窄）。",
    "lib.rs::pub const fn narrow(ch: char, style: u16) -> Self {": "构造一个窄字符单元格。",
    "lib.rs::pub const fn wide_head(ch: char, style: u16) -> Self {": "构造宽字符的左半格。",
    "lib.rs::pub const fn wide_tail(style: u16) -> Self {": "构造宽字符的右半格（无字符，仅占位）。",
    "lib.rs::pub line: LineId,": "本行所属的逻辑行 id（软换行时多行共享同一个 `LineId`）。",
    "lib.rs::pub struct Row {": (
        "屏幕上一行。与\"逻辑行\"的区别：终端里一行长文本会软换行成多行 `Row`，"
        "它们共享 `LineId` 与 `line_start`，这样重排（reflow）时才能还原成一段。"
    ),
    "lib.rs::pub line: LineId,": "所属逻辑行。",
    "lib.rs::pub fn new(cols: u16, line: LineId, line_start: u32) -> Self {": "建一个空行；`line_start` 是本行在逻辑行里的字符偏移。",
    "lib.rs::pub struct Grid {": "终端网格：屏幕行 + 历史行 + 光标与选中锚点。",
    "lib.rs::pub cols: u16,": "列数。",
    "lib.rs::pub rows: u16,": "屏幕可见行数（不含历史）。",
    "lib.rs::pub screen: Vec<Row>,": "当前屏幕的可见行。",
    "lib.rs::pub cursor_row: usize,": "光标所在屏幕行。",
    "lib.rs::pub cursor_col: usize,": "光标所在列。",
    "lib.rs::pub pending_wrap: bool,": (
        "是否处于\"待换行\"状态：写满最后一列后，光标停在行尾而不是立刻跳到下一行，"
        "等下一个字符到来才真正换行。否则行尾字符会提前把光标顶走。"
    ),
    "lib.rs::pub anchor: CursorAnchor,": "选区锚点（起始单元格）。",
    "lib.rs::pub history_limit: usize,": "历史行上限；超出时最早的行被丢弃，防止长时间运行内存无上限增长。",
    "lib.rs::pub fn new(cols: u16, rows: u16) -> Self {": "建一个空网格；`cols`/`rows` 至少为 1。",
    "lib.rs::pub fn scroll_bottom(&mut self) {": (
        "把视口滚到最底部（新输出到来时调用）。为什么单独一个函数："
        "滚动位置与光标位置是两件事——用户向上翻历史时光标仍在最后一行。"
    ),
    # ---------------- y901-backend-win32 ----------------
    "mod.rs::pub struct D3dBackend {": "D3D11 渲染后端：持有设备、交换链与实例缓冲；它不认识 UI 树，只认实例。",
    "mod.rs::pub gfx: Gfx,": "D3D11 设备与交换链。设备丢失后由 `Gfx::rebuild` 重建，因此这里是可重建的唯一入口。",
    "mod.rs::pub fn live_objects() -> isize {": (
        "当前存活的 COM 对象计数（只增不减说明有泄漏）。测试用它断言\"用完全部释放\"，"
        "比肉眼看画面可靠。"
    ),
    "d3d11.rs::pub type Hresult = i32;": "HRESULT；负值表示失败。",
    "d3d11.rs::pub const INSTANCE_SIZE: usize = 96;": (
        "单实例字节数，**必须与 `create_pipeline` 里的 InputLayout 完全一致**："
        "改一个字段而忘了改布局，整帧会变成乱码。"
    ),
    "d3d11.rs::pub const INSTANCE_CAPACITY: usize = 10_000;": (
        "单帧实例上限。超出时按此截断而不是扩缓冲——每帧重分配缓冲比少画几个元素更糟。"
    ),
    "d3d11.rs::pub pos: [f32; 2],": "左上角像素坐标。",
    "d3d11.rs::pub size: [f32; 2],": "像素尺寸。",
    "d3d11.rs::pub uv_rect: [f32; 4],": (
        "图集 UV。`xy == zw` 表示纯色/渐变（不采样），这是纯色矩形走同一管线的关键："
        "少一个分支就少一次状态切换。"
    ),
    "d3d11.rs::pub color: [f32; 4],": "主色（渐变起点、字形颜色）。",
    "d3d11.rs::pub color_end: [f32; 4],": "渐变终点色；纯色实例与 `color` 相同。",
    "d3d11.rs::pub gradient: [f32; 4],": "`(方向 x, 方向 y, 是否渐变, 偏移)`。",
    "d3d11.rs::pub left: i32,": "矩形左边界（含）。",
    "d3d11.rs::pub top: i32,": "矩形上边界（含）。",
    "d3d11.rs::pub right: i32,": "矩形右边界（不含）——半开区间避免相邻脏区重复绘制同一列像素。",
    "d3d11.rs::pub bottom: i32,": "矩形下边界（不含）。",
    "d3d11.rs::pub struct Gfx {": "D3D11 管线集合：设备、上下文、交换链、自持渲染目标与字形图集。",
    "d3d11.rs::pub fn is_empty(&self) -> bool {": "是否尚未初始化（创建失败或设备丢失后为 true，调用方据此走重建路径）。",
    "d3d11.rs::pub fn device_lost(hr: Hresult) -> bool {": (
        "失败的 HRESULT 是否表示设备被移除/重置。这类失败是**可恢复**的：重建后继续画；"
        "其它失败则应当上报。"
    ),
    "d3d11.rs::pub fn is_occluded(hr: Hresult) -> bool {": (
        "`Present` 的返回是否表示窗口被遮挡。被遮挡时不必重画——这正是空闲时省 CPU 的一半来源。"
    ),
    "font.rs::pub type Hresult = i32;": "HRESULT；负值表示失败。",
    "font.rs::pub struct TextEngine {": (
        "DirectWrite 字形引擎（Windows 原生实现）。与自研 `MyFontEngine` 实现同一个 `FontEngine`，"
        "两者可互换而上层一行不改。"
    ),
    "font.rs::pub atlas: Vec<u8>,": "字形图集像素（RGBA：`rgb` 存覆盖率，`a` 恒为 255）。",
    "font.rs::pub atlas_size: u32,": "图集边长（像素）。",
    "font.rs::pub ascent: f32,": "基线到行顶的距离；绘制时基线的位置由它决定。",
    "font.rs::pub descent: f32,": "基线到行底的距离；与 `ascent` 一起决定行高。",
    "font.rs::pub cell_width: f32,": "等宽单元格宽度（由字号推出，必须与布局用的一致）。",
    "font.rs::pub cell_height: f32,": "等宽单元格高度。",
    "font.rs::pub fn glyph_count(&self) -> usize {": "已缓存字形数（诊断与内存估算用）。",
    "font.rs::pub fn has_dirty(&self) -> bool {": "是否有待上传的图集脏区；false 时后端可跳过整张图集上传。",
    "ime.rs::pub const WM_IME_STARTCOMPOSITION: u32 = 0x010D;": "开始组字（输入法开始拼一个候选串）。",
    "ime.rs::pub const WM_IME_ENDCOMPOSITION: u32 = 0x010E;": "结束组字。",
    "ime.rs::pub const WM_IME_COMPOSITION: u32 = 0x010F;": "组字更新；`lParam` 的 `GCS_*` 位标出本次携带了什么。",
    "ime.rs::pub const GCS_COMPSTR: u32 = 0x0008;": "本次携带的是\"正在组字的中间串\"（预编辑文本）。",
    "ime.rs::pub const GCS_RESULTSTR: u32 = 0x0800;": "本次携带的是\"已确定的最终串\"，应当作为 `UiEvent::Text` 提交。",
    "ime.rs::pub const GCS_CURSORPOS: u32 = 0x0080;": "本次携带了候选串里的光标位置（用于在预编辑串中画插入符）。",
    "ime.rs::pub fn composing_text(&self) -> String {": (
        "当前预编辑串。为什么与\"已提交文本\"分开：预编辑只画在覆盖层，"
        "绝不进网格——否则撤销一次组字就要回滚整屏。"
    ),
    "ime.rs::pub fn clear(&mut self) {": "清空预编辑状态（组字被取消、或焦点离开输入框时调用）。",
    "window.rs::pub fn create(options: &Options) -> Result<Self, String> {": (
        "创建窗口与渲染后端。`Options.width/height` 是**窗口尺寸**（含边框与标题栏），"
        "实际渲染用的是 `GetClientRect` 得到的客户区，后者会小一圈。"
    ),
    "window.rs::pub fn viewport(&self) -> (u32, u32) {": "当前客户区尺寸（像素）；布局与渲染都以它为准。",
    "window.rs::pub fn close(&self) {": (
        "请求关闭窗口（投递退出消息）。为什么不是析构：关闭要经过消息循环，"
        "直接销毁会让正在处理的这条消息访问悬垂句柄。"
    ),
}


def main() -> int:
    fill_docs.DOCS.update(EXTRA)
    original = fill_docs.DOCS.get

    def lookup(snip: str):
        return fill_docs.DOCS.get(snip)

    # 让 fill_docs 按"文件::片段"查找：替换其 patch 内部逻辑
    orig_patch = fill_docs.patch

    def patch(path: str, lineno: int):
        with io.open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
        idx = lineno - 1
        if idx >= len(lines):
            return None
        if idx > 0 and lines[idx - 1].strip().startswith("///"):
            return None
        snip = lines[idx].strip()
        base = path.replace("\\", "/").rsplit("/", 1)[-1]
        doc = EXTRA.get(f"{base}::{snip}")
        if doc is None:
            return None
        indent = lines[idx][: len(lines[idx]) - len(lines[idx].lstrip())]
        block = [f"{indent}/// {p}" if p else f"{indent}///" for p in doc.split("\n")]
        lines[idx:idx] = block
        with io.open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(lines))
        return doc

    fill_docs.patch = patch
    return fill_docs.main()


if __name__ == "__main__":
    sys.exit(main())
