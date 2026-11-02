#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""第 4 轮：y901-ui 的 Widget / RenderCtx / UiTree 及其支撑类型（约 75 项）。

锚点是**语义的**：`文件 + 完整声明行文本`（不是行号——行号会随插入漂移，这是第 3 轮的教训）。
"""

import io
import subprocess
import sys

ROOT = "F:/Y901"
FILES = {
    "widget": "crates/y901-ui/src/widget.rs",
    "tree": "crates/y901-ui/src/tree.rs",
}

DOCS: dict[str, str] = {
    # ================= widget.rs =================
    "widget::srcdoc": "",  # 占位，避免误用
    "widget::Default,": "默认输入类型：只声明动作名，不约定参数形状。",
    "widget::pub fn new(id: impl Into<String>, invalidation: crate::signal::Invalidation) -> Self {":
        "建立挂载上下文。`invalidation` 是后台任务请求重绘的通道——组件挂载时拿到它，"
        "就能在不持有 `UiTree` 的情况下通知界面刷新。",
    "widget::pub const PANEL: Role = Role(\"panel\");": "容器面板；也是未知角色的回退槽位。",
    "widget::pub const WINDOW: Role = Role(\"window\");": "窗口背景（与面板分开：窗口色通常是整屏底色）。",
    "widget::pub const TEXT: Role = Role(\"text\");": "普通文本。",
    "widget::pub const BUTTON: Role = Role(\"button\");": "按钮；同时被用作\"可交互块\"的通用外观。",
    "widget::pub const TEXT_INPUT: Role = Role(\"text_input\");": "文本输入框。",
    "widget::pub const LIST: Role = Role(\"list\");": "列表容器。",
    "widget::pub const LIST_ITEM: Role = Role(\"list_item\");": "列表项（与容器分开：选中态通常只作用于项）。",
    "widget::pub const SCROLLBAR: Role = Role(\"scrollbar\");": "滚动条。",
    "widget::pub const fn as_str(self) -> &'static str {": "角色名；语义树与主题查表都用它作为稳定标识。",
    "widget::Horizontal,": "水平主轴（子控件从左到右排列）。",
    "widget::Vertical,": "垂直主轴（从上到下）。",
    "widget::pub fn resolve(&self, available: f32, content: f32) -> f32 {":
        "把\"可用空间 + 内容尺寸\"解析成最终长度。为什么需要它：`Auto` 要知道内容多大、"
        "`Fill` 要吃掉剩余、`Fixed` 无视两者——三者必须用同一处逻辑，否则父容器各写一遍就会不一致。",
    "widget::pub fn flex_factor(&self) -> Option<f32> {":
        "弹性权重（`Flex(f)` 返回 `Some(f)`，其余 `None`）。父容器据此分配剩余空间。",
    "widget::pub fn is_flex(&self) -> bool {": "是否是弹性尺寸（`Fill` 或 `Flex`）。",
    "widget::pub width: Dimension,": "宽度约束。",
    "widget::pub height: Dimension,": "高度约束。",
    "widget::pub const fn fill() -> Self {": "两个方向都填满可用空间（最常用）。",
    "widget::pub const fn width(w: f32) -> Self {": "固定宽度，高度填满。",
    "widget::pub const fn height(h: f32) -> Self {": "固定高度，宽度填满。",
    "widget::pub const fn grow(factor: f32) -> Self {": "按权重瓜分剩余空间（`factor` 越大占比越高）。",
    "widget::pub const NONE: Axis2 = Axis2(0);": "无轴。",
    "widget::pub const H: Axis2 = Axis2(1);": "仅水平轴。",
    "widget::pub const V: Axis2 = Axis2(2);": "仅垂直轴。",
    "widget::pub const BOTH: Axis2 = Axis2(3);": "两个轴。",
    "widget::pub const fn contains(self, other: Axis2) -> bool {": "是否包含某个轴（位运算，用于约束传播）。",
    "widget::pub const fn union(self, other: Axis2) -> Axis2 {": "合并两个轴集合。",
    "widget::pub const fn any(self) -> bool {": "是否至少含一个轴。",
    "widget::pub max: Size,": "尺寸上限。`f32::INFINITY` 表示该方向不设限（滚动内容常用）。",
    "widget::pub min: Size,": "尺寸下限；布局不会把子控件压到比它更小。",
    "widget::pub unbounded: Axis2,": "哪些方向是无界的。为什么单独一个字段：`max` 里的 `INFINITY` 无法区分"
        "\"父容器允许无限\"与\"测量时人为放开\"，布局需要知道是哪种。",
    "widget::pub const fn loose(max: Size) -> Self {": "宽松约束：上界给定，下界为 0（子控件可以更小）。",
    "widget::pub const fn tight(size: Size) -> Self {": "紧约束：上下界都是 `size`（子控件必须正好这么大）。",
    "widget::pub fn is_unbounded(&self, axis: crate::Axis) -> bool {": "该方向是否无界（滚动容器据此决定要不要撑开）。",
    "widget::pub trait RenderCtx: TextMetrics {":
        "渲染上下文：框架唯一的出图接口。后端实现它（D3D11 / 软件 / 测试记录器），组件只往里画。"
        "为什么继承 `TextMetrics`：测量与绘制必须用**同一套字体度量**，否则文字会与布局错位。",
    "widget::fn fill_rect(&mut self, rect: Rect, color: Color);":
        "填充一个轴对齐矩形（屏幕坐标，像素对齐）。alpha < 1 时与已有内容混合。",
    "widget::fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);":
        "画矩形边框。`width` 向内收：边框完全落在 `rect` 内，不会撑大占位。",
    "widget::fn pop_clip(&mut self);":
        "弹出最近一次 `push_clip`。必须与 push 严格配对——不配对会让后续绘制被错误裁剪。",
    "widget::pub viewport: Size,": "本帧的视口尺寸；事件坐标换算与边界判断都用它。",
    "widget::pub fn new(focus: Option<String>, viewport: Size) -> Self {":
        "建立事件上下文。`focus` 是当前焦点节点（组件据此判断自己是否持有键盘），"
        "`viewport` 用于把点击位置换算成布局坐标。",
    "widget::pub fn request_focus(&mut self, id: impl Into<String>) {":
        "请求把键盘焦点交给某个节点。为什么要\"请求\"而不是直接改：焦点是**树的状态**，"
        "必须在分发结束后统一落实，否则同一次事件里两个组件会各自以为自己拿到了焦点。",
    "widget::pub fn request_redraw(&mut self) {": "请求重绘（脏区由树在分发后统一标）。",
    "widget::pub fn request_relayout(&mut self) {":
        "请求重排。只有尺寸/结构真的变了才该用它——它会触发整棵子树的测量。",
    "widget::pub fn is_focused(&self, id: &str) -> bool {": "某个节点是否持有键盘焦点。",
    "widget::pub fn finish(&mut self, result: EventResult) -> EventResult {":
        "把组件返回值与本次累积的\"请求重绘/重排\"合并。组件不需要自己拼 `EventResult`。",
    "widget::None,": "无参数（纯动作，例如\"关闭\"）。",
    "widget::Text,": "文本参数。",
    "widget::Number,": "数值参数。",
    "widget::Bool,": "布尔参数。",
    "widget::pub name: String,": "动作名。",
    "widget::pub input: InputKind,": "参数形状；网关据此做类型校验，避免把字符串传进数值参数。",
    "widget::pub label: String,": "给人和 Agent 看的文字标签。",
    "widget::pub value: String,": "当前值的字符串形式（Agent 直接消费，不做类型推断）。",
    "widget::pub fn label(mut self, label: impl Into<String>) -> Self {": "设置标签。",
    "widget::pub fn value(mut self, value: impl Into<String>) -> Self {": "设置当前值。",
    "widget::pub fn disabled(mut self, disabled: bool) -> Self {": "标记为禁用；网关据此拒绝在其上执行动作。",
    "widget::pub fn prop(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {":
        "追加一项自定义属性。为什么用扁平键值而不是结构体：组件自定义的属性无法事先枚举。",
    "widget::fn clip(&self) -> Option<Rect> {":
        "本节点及其子树的裁剪矩形（`None` = 不裁剪）。滚动容器用它把内容裁到视口内——"
        "与 `children_clip` 的区别是后者只作用于**子控件各自**的格子。",

    # ================= tree.rs =================
    "tree::pub id: String,": "节点 id（树内唯一）。",
    "tree::pub role: Role,": "角色名；主题查表与语义过滤都用它。",
    "tree::pub rect: Rect,": "布局后的屏幕矩形。",
    "tree::pub focused: bool,": "是否持有键盘焦点。",
    "tree::pub semantics: Semantics,": "语义信息（标签、值、动作）；这就是 Agent 看到的内容。",
    "tree::pub children: Vec<NodeInfo>,": "子节点。",
    "tree::pub fn add(&mut self, rect: Rect) {":
        "追加一块脏区。为什么不合并成包围盒：两块不相邻的脏区合并后，"
        "中间的空白也会被清屏——而 CPU 侧已跳过那些节点，结果就是一块黑。",
    "tree::pub fn is_full(&self) -> bool {": "是否整窗重绘（此时 `rects()` 无意义）。",
    "tree::pub fn is_empty(&self) -> bool {": "是否没有脏区（可以整帧跳过绘制）。",
    "tree::pub fn len(&self) -> usize {": "脏区块数。",
    "tree::pub fn clear(&mut self) {": "清空脏区（渲染完成后调用）。",
    "tree::pub struct UiTree {":
        "UI 树：持有根节点、索引、焦点、脏区、浮层与动画。**所有状态变更都从这里发起**，"
        "组件本身不持有跨帧状态。",
    "tree::pub fn use_state<T: Clone + PartialEq + 'static>(":
        "声明一段随树存活的状态，返回可读写的句柄。为什么不用 `Rc<RefCell>`："
        "句柄变更会登记到树，树据此标脏；手写 `Rc` 就绕过了这套通知。",
    "tree::pub fn close_popup(&mut self, id: &str) -> bool {":
        "关闭指定浮层；不存在返回 `false`（不 panic）。控件应通过 `EventCtx::request_close_popup` 调用，"
        "让关闭发生在分发结束后。",
    "tree::pub fn remove_node(&mut self, id: &str) -> bool {":
        "移除节点及其子树。返回 `false` 表示根节点或节点不存在。"
        "注意：会触发卸载钩子（`release_textures` / `on_unmount`），纹理因此被回收。",
    "tree::pub fn find_path(&self, id: &str) -> Option<Vec<usize>> {":
        "按 id 求从根到该节点的子索引路径。为什么用路径而不是指针：路径可跨帧保存，"
        "且能在重排后重新定位（指针在树变动后会失效）。",
}


def collect() -> list[tuple[str, int, str]]:
    out = subprocess.run(
        ["cargo", "build", "-p", "y901-ui", "--offline"],
        cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace",
    ).stderr
    items, loc = [], None
    for line in out.splitlines():
        if "missing documentation for " in line:
            loc = None
            continue
        if "-->" in line and loc is None:
            part = line.split("-->", 1)[1].strip()
            path, _, rest = part.partition(":")
            items.append((path, int(rest.split(":")[0])))
    return items


def main() -> int:
    items = collect()
    items = [(p, n) for p, n in items if p.replace("\\", "/").endswith(("widget.rs", "tree.rs"))]
    items.sort(key=lambda t: (t[0], -t[1]))
    done = skipped = 0
    for path, lineno in items:
        full = f"{ROOT}/{path.replace(chr(92), '/')}"
        base = full.rsplit("/", 1)[-1].replace(".rs", "")
        with io.open(full, encoding="utf-8") as f:
            lines = f.read().split("\n")
        idx = lineno - 1
        if idx >= len(lines) or (idx > 0 and lines[idx - 1].strip().startswith("///")):
            skipped += 1
            continue
        snip = lines[idx].strip()
        doc = DOCS.get(f"{base}::{snip}")
        if doc is None:
            skipped += 1
            print(f"[skip] {base}:{lineno} :: {snip[:70]}")
            continue
        indent = lines[idx][: len(lines[idx]) - len(lines[idx].lstrip())]
        block = [f"{indent}/// {p}" if p else f"{indent}///" for p in doc.split("\n")]
        lines[idx:idx] = block
        with io.open(full, "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(lines))
        done += 1
    print(f"插入 {done} 处，跳过 {skipped} 处")
    return 0


if __name__ == "__main__":
    sys.exit(main())
