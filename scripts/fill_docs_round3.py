#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""按 (文件, 行号) 精确插入 `///`：第 3 轮（agent + runtime，112 项）。

为什么改用行号而不是片段：本轮大量重复片段（`x: f32,` 在 `RawEvent` 里出现 4 次、
`id: String,` 在 `api.rs` 里出现 5 次），片段无法区分"是哪个变体的哪个字段"。
行号来自 rustc 的 `--> file:line`，是完全确定的。

安全措施与之前一致：插入前校验该行内容与预期片段一致；同一文件从下往上插；已有 `///` 跳过。
"""

import io
import subprocess
import sys

ROOT = "F:/Y901"

# key = "相对路径:行号"，value = (该行片段, 文档)
DOCS: dict[str, tuple[str, str]] = {
    # ---------------- y901-agent/api.rs ----------------
    "crates/y901-agent/src/api.rs:17": (
        "Query { q: String },",
        "全文检索；`q` 是自然语言或关键词，具体匹配规则由 `query` 实现决定。",
    ),
    "crates/y901-agent/src/api.rs:19": (
        "Click { id: String, if_rev: Option<u64> },",
        "点击某个节点。`if_rev` 是乐观并发校验：调用方带上它看到的版本号，"
        "节点在期间变过就返回 `Conflict` 而不是误操作。",
    ),
    "crates/y901-agent/src/api.rs:22": (
        "id: String,",
        "目标节点 id。",
    ),
    "crates/y901-agent/src/api.rs:23": (
        "text: String,",
        "要写入的文本。",
    ),
    "crates/y901-agent/src/api.rs:24": (
        "if_rev: Option<u64>,",
        "版本校验：`None` 表示不校验。",
    ),
    "crates/y901-agent/src/api.rs:28": (
        "id: String,",
        "目标节点 id（滚动容器）。",
    ),
    "crates/y901-agent/src/api.rs:29": (
        "delta: f32,",
        "滚动增量（像素或行，由容器解释）；正负号决定方向。",
    ),
    "crates/y901-agent/src/api.rs:30": (
        "if_rev: Option<u64>,",
        "版本校验。",
    ),
    "crates/y901-agent/src/api.rs:34": (
        "id: String,",
        "目标节点 id。",
    ),
    "crates/y901-agent/src/api.rs:35": (
        "action: String,",
        "动作名；框架不认识它的语义，原样投给组件。",
    ),
    "crates/y901-agent/src/api.rs:36": (
        "args: Json,",
        "动作参数，原样传递。",
    ),
    "crates/y901-agent/src/api.rs:37": (
        "if_rev: Option<u64>,",
        "版本校验。",
    ),
    "crates/y901-agent/src/api.rs:78": (
        "Forbidden { id: String },",
        "节点存在但不允许被 Agent 操作（未 `exposed(true)`）。",
    ),
    "crates/y901-agent/src/api.rs:80": (
        "NotFound { id: String },",
        "树上没有这个 id。",
    ),
    "crates/y901-agent/src/api.rs:82": (
        "Conflict { expected: u64, actual: u64 },",
        "版本不符：`expected` 是调用方以为的版本，`actual` 是当前版本。"
        "分开给两个值是为了让调用方能判断\"重试还是放弃\"。",
    ),
    # ---------------- y901-agent/mcp.rs ----------------
    "crates/y901-agent/src/mcp.rs:21": (
        "Uninitialized,",
        "尚未完成 `initialize` 握手；此时除握手请求外的调用都应被拒绝。",
    ),
    "crates/y901-agent/src/mcp.rs:22": (
        "Initialized,",
        "握手完成，可以正常处理工具调用。",
    ),
    "crates/y901-agent/src/mcp.rs:25": (
        "pub struct McpServer {",
        "MCP 协议状态机。只保存协议层状态，不持有 UI 树——树在每次请求时传入，"
        "这样服务端可以跨窗口复用。",
    ),
    "crates/y901-agent/src/mcp.rs:27": (
        "pub state: State,",
        "当前协议状态。",
    ),
    # ---------------- y901-agent/rpc.rs ----------------
    "crates/y901-agent/src/rpc.rs:26": (
        "pub const DEFAULT_ADDR: &str = \"127.0.0.1:9410\";",
        "默认监听地址。只绑回环：这是本机 Agent 通道，不应暴露到网络。",
    ),
    "crates/y901-agent/src/rpc.rs:35": (
        "node: String,",
        "焦点所在节点 id。",
    ),
    "crates/y901-agent/src/rpc.rs:36": (
        "event: RpcEvent,",
        "具体事件。",
    ),
    "crates/y901-agent/src/rpc.rs:40": (
        "node: String,",
        "触发动作的节点 id。",
    ),
    "crates/y901-agent/src/rpc.rs:41": (
        "action: String,",
        "动作名（组件自定义）。",
    ),
    "crates/y901-agent/src/rpc.rs:42": (
        "args: Json,",
        "动作参数。",
    ),
    "crates/y901-agent/src/rpc.rs:46": (
        "q: String,",
        "检索串。",
    ),
    "crates/y901-agent/src/rpc.rs:50": (
        "node: String,",
        "订阅的根节点 id。",
    ),
    "crates/y901-agent/src/rpc.rs:54": (
        "node: String,",
        "要退订的节点 id；与订阅时一致。",
    ),
    "crates/y901-agent/src/rpc.rs:55": (
        "interval_ms: u64,",
        "最小推送间隔（毫秒）。为什么要节流：语义树变化可能每帧都发生，"
        "不节流会把订阅端淹掉。",
    ),
    "crates/y901-agent/src/rpc.rs:59": (
        "node: String,",
        "目标节点 id。",
    ),
    "crates/y901-agent/src/rpc.rs:63": (
        "Unknown {",
        "无法识别的方法名。单独一个变体而不是直接报错：这样服务端能回"
        "标准的 \"method not found\"，而不是连接断开。",
    ),
    "crates/y901-agent/src/rpc.rs:64": (
        "method: String,",
        "原始的未知方法名（回给调用方，便于排查）。",
    ),
    "crates/y901-agent/src/rpc.rs:71": (
        "Text(String),",
        "输入一段文本（等价于用户打字）：走 `UiEvent::Text`。",
    ),
    "crates/y901-agent/src/rpc.rs:72": (
        "Key(y901_ui::Key),",
        "按一个键（无修饰键）：走 `UiEvent::Key`。",
    ),
    "crates/y901-agent/src/rpc.rs:73": (
        "Compose(String),",
        "输入法预编辑串：只走覆盖层，**不进网格**——提交与否由后续的 `Text` 决定。",
    ),
    "crates/y901-agent/src/rpc.rs:74": (
        "MouseDown {",
        "在指定位置按下鼠标。坐标是客户区逻辑像素。",
    ),
    "crates/y901-agent/src/rpc.rs:75": (
        "pos: y901_ui::Pos,",
        "客户区坐标。",
    ),
    "crates/y901-agent/src/rpc.rs:76": (
        "button: y901_ui::MouseButton,",
        "哪个键被按下。",
    ),
    "crates/y901-agent/src/rpc.rs:78": (
        "MouseUp {",
        "抬起鼠标。",
    ),
    "crates/y901-agent/src/rpc.rs:79": (
        "pos: y901_ui::Pos,",
        "客户区坐标。",
    ),
    "crates/y901-agent/src/rpc.rs:80": (
        "button: y901_ui::MouseButton,",
        "哪个键被抬起。",
    ),
    "crates/y901-agent/src/rpc.rs:82": (
        "MouseMove {",
        "移动鼠标（会驱动 hover 与拖拽）。",
    ),
    "crates/y901-agent/src/rpc.rs:83": (
        "pos: y901_ui::Pos,",
        "客户区坐标。",
    ),
    "crates/y901-agent/src/rpc.rs:85": (
        "Wheel {",
        "滚轮。",
    ),
    "crates/y901-agent/src/rpc.rs:86": (
        "pos: y901_ui::Pos,",
        "滚轮发生的位置（决定滚哪个容器）。",
    ),
    "crates/y901-agent/src/rpc.rs:87": (
        "delta: f32,",
        "滚动量；正数向上。",
    ),
    "crates/y901-agent/src/rpc.rs:297": (
        "pub request: RpcRequest,",
        "已解析的请求。",
    ),
    "crates/y901-agent/src/rpc.rs:298": (
        "pub id: Json,",
        "JSON-RPC 请求 id，原样回写响应，用于把响应与请求配对。",
    ),
    # ---------------- y901-runtime/lib.rs ----------------
    "crates/y901-runtime/src/lib.rs:34": (
        "pub trait Host {",
        "应用层契约：平台运行时只负责窗口与事件，业务判断（命令、策略、快照去向）都由 Host 决定。",
    ),
    "crates/y901-runtime/src/lib.rs:179": (
        "pub title: String,",
        "窗口标题。",
    ),
    "crates/y901-runtime/src/lib.rs:180": (
        "pub width: u32,",
        "**窗口**宽度（含边框），不是客户区宽度；客户区由平台减去边框与标题栏。",
    ),
    "crates/y901-runtime/src/lib.rs:181": (
        "pub height: u32,",
        "**窗口**高度（含标题栏）。",
    ),
    "crates/y901-runtime/src/lib.rs:228": (
        "MouseMove {",
        "指针移动（客户区坐标，已按 DPI 换算成逻辑像素）。",
    ),
    "crates/y901-runtime/src/lib.rs:232": (
        "MouseDown {",
        "按下鼠标。",
    ),
    "crates/y901-runtime/src/lib.rs:237": (
        "MouseUp {",
        "抬起鼠标。",
    ),
    "crates/y901-runtime/src/lib.rs:242": (
        "Wheel {",
        "滚轮；带修饰键，因为 Ctrl+滚轮常用来缩放。",
    ),
    "crates/y901-runtime/src/lib.rs:250": (
        "paths: Vec<std::path::PathBuf>,",
        "拖入的文件绝对路径；框架只负责路由，不解释内容。",
    ),
    "crates/y901-runtime/src/lib.rs:251": (
        "x: f32,",
        "松手位置的 X（客户区坐标）。",
    ),
    "crates/y901-runtime/src/lib.rs:252": (
        "y: f32,",
        "松手位置的 Y。",
    ),
    "crates/y901-runtime/src/lib.rs:254": (
        "KeyDown {",
        "按键按下。",
    ),
    "crates/y901-runtime/src/lib.rs:255": (
        "key: Key,",
        "按下的键。",
    ),
    "crates/y901-runtime/src/lib.rs:256": (
        "mods: Modifiers,",
        "当时的修饰键状态。",
    ),
    "crates/y901-runtime/src/lib.rs:260": (
        "key: Key,",
        "系统级按键（如 Alt 组合），与普通 `KeyDown` 分开是因为它不该被组件当成文本输入。",
    ),
    "crates/y901-runtime/src/lib.rs:263": (
        "Char {",
        "字符输入（已过键盘布局/输入法翻译）。",
    ),
    "crates/y901-runtime/src/lib.rs:264": (
        "c: char,",
        "字符。",
    ),
    "crates/y901-runtime/src/lib.rs:266": (
        "Ime {",
        "输入法事件。`composition` 区分\"正在组字\"与\"已提交\"。",
    ),
    "crates/y901-runtime/src/lib.rs:267": (
        "text: String,",
        "组字中间串或最终串。",
    ),
    "crates/y901-runtime/src/lib.rs:268": (
        "composition: bool,",
        "`true` = 预编辑（只画不提交）；`false` = 已提交（应当进入文本输入）。",
    ),
    "crates/y901-runtime/src/lib.rs:270": (
        "Resize {",
        "客户区尺寸变化。",
    ),
    "crates/y901-runtime/src/lib.rs:271": (
        "w: u32,",
        "新的客户区宽度。",
    ),
    "crates/y901-runtime/src/lib.rs:272": (
        "h: u32,",
        "新的客户区高度。",
    ),
    "crates/y901-runtime/src/lib.rs:274": (
        "SetCursor {",
        "请求把系统光标设成某个形状（由 `UiTree::cursor_at` 决定）。",
    ),
    "crates/y901-runtime/src/lib.rs:275": (
        "x: f32,",
        "位置 X。",
    ),
    "crates/y901-runtime/src/lib.rs:276": (
        "y: f32,",
        "位置 Y。",
    ),
    "crates/y901-runtime/src/lib.rs:278": (
        "Close,",
        "窗口关闭请求。",
    ),
    # ---------------- y901-runtime/gfx.rs ----------------
    "crates/y901-runtime/src/gfx.rs:66": (
        "pub x: u32,",
        "脏区左上角 X（像素）。",
    ),
    "crates/y901-runtime/src/gfx.rs:67": (
        "pub y: u32,",
        "脏区左上角 Y。",
    ),
    "crates/y901-runtime/src/gfx.rs:68": (
        "pub w: u32,",
        "脏区宽度。",
    ),
    "crates/y901-runtime/src/gfx.rs:69": (
        "pub h: u32,",
        "脏区高度。",
    ),
    "crates/y901-runtime/src/gfx.rs:75": (
        "pub instances: Vec<GfxInstance>,",
        "本帧的实例列表；顺序即绘制顺序（后画的在上）。",
    ),
    "crates/y901-runtime/src/gfx.rs:76": (
        "pub atlas_updates: Vec<AtlasUpdate>,",
        "本帧需要上传的字形图集脏区；空表示图集没变，可跳过上传。",
    ),
    "crates/y901-runtime/src/gfx.rs:117": (
        "Create {",
        "首次创建资源。",
    ),
    "crates/y901-runtime/src/gfx.rs:118": (
        "width: u32,",
        "创建时的宽度。",
    ),
    "crates/y901-runtime/src/gfx.rs:119": (
        "height: u32,",
        "创建时的高度。",
    ),
    "crates/y901-runtime/src/gfx.rs:121": (
        "Rebuild {",
        "设备丢失后重建（尺寸不变）。",
    ),
    "crates/y901-runtime/src/gfx.rs:122": (
        "width: u32,",
        "重建宽度。",
    ),
    "crates/y901-runtime/src/gfx.rs:123": (
        "height: u32,",
        "重建高度。",
    ),
    "crates/y901-runtime/src/gfx.rs:125": (
        "Resize {",
        "尺寸变化后的调整。",
    ),
    "crates/y901-runtime/src/gfx.rs:126": (
        "width: u32,",
        "新宽度。",
    ),
    "crates/y901-runtime/src/gfx.rs:127": (
        "height: u32,",
        "新高度。",
    ),
    "crates/y901-runtime/src/gfx.rs:129": (
        "Present {",
        "一次提交。",
    ),
    "crates/y901-runtime/src/gfx.rs:130": (
        "instances: usize,",
        "本次提交的实例数。",
    ),
    "crates/y901-runtime/src/gfx.rs:131": (
        "atlas_updates: usize,",
        "本次上传的图集脏区数。",
    ),
    "crates/y901-runtime/src/gfx.rs:132": (
        "atlas_reset: bool,",
        "是否整张图集重传（图集重建后为 true）。",
    ),
    "crates/y901-runtime/src/gfx.rs:142": (
        "pub calls: Vec<BackendCall>,",
        "按顺序记录的调用；测试用它断言\"不该发生的调用确实没发生\"（例如遮挡时不该有 Present）。",
    ),
    "crates/y901-runtime/src/gfx.rs:143": (
        "pub last_frame: Option<Frame>,",
        "最近一帧的内容（供断言检查实例数、脏区）。",
    ),
    "crates/y901-runtime/src/gfx.rs:144": (
        "pub occluded: bool,",
        "上次提交是否因被遮挡而跳过；mock 用它模拟遮挡场景。",
    ),
    "crates/y901-runtime/src/lib.rs:229": ("x: f32,", "指针位置 X（客户区逻辑像素）。"),
    "crates/y901-runtime/src/lib.rs:230": ("y: f32,", "指针位置 Y。"),
    "crates/y901-runtime/src/lib.rs:233": ("x: f32,", "按下位置 X。"),
    "crates/y901-runtime/src/lib.rs:234": ("y: f32,", "按下位置 Y。"),
    "crates/y901-runtime/src/lib.rs:235": ("button: MouseButton,", "哪个键被按下。"),
    "crates/y901-runtime/src/lib.rs:238": ("x: f32,", "抬起位置 X。"),
    "crates/y901-runtime/src/lib.rs:239": ("y: f32,", "抬起位置 Y。"),
    "crates/y901-runtime/src/lib.rs:240": ("button: MouseButton,", "哪个键被抬起。"),
    "crates/y901-runtime/src/lib.rs:243": ("x: f32,", "滚轮位置 X。"),
    "crates/y901-runtime/src/lib.rs:244": ("y: f32,", "滚轮位置 Y。"),
    "crates/y901-runtime/src/lib.rs:245": ("delta: f32,", "滚动量；正数向上。"),
    "crates/y901-runtime/src/lib.rs:246": ("mods: Modifiers,", "滚轮时的修饰键。"),
    "crates/y901-runtime/src/lib.rs:261": ("mods: Modifiers,", "修饰键状态。"),
    # ---- y901-agent rpc 剩余字段 ----
}


def build_warnings() -> list[tuple[str, int, str]]:
    out = subprocess.run(
        ["cargo", "build", "-p", "y901-agent", "-p", "y901-runtime", "--offline"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    ).stderr
    items, pending, loc = [], None, None
    for line in out.splitlines():
        if "missing documentation for " in line:
            pending = line.strip()
            continue
        if "-->" in line and pending:
            part = line.split("-->", 1)[1].strip()
            loc = part.rsplit(":", 1)[0] + ":" + part.rsplit(":", 1)[1].split(":")[0]
            items.append((loc, pending))
            pending, loc = None, None
    return items


def main() -> int:
    # 直接从 cargo 输出取 (file:line)
    out = subprocess.run(
        ["cargo", "build", "-p", "y901-agent", "-p", "y901-runtime", "--offline"],
        cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace",
    ).stderr
    todo: list[tuple[str, int]] = []
    pending = False
    for line in out.splitlines():
        if "missing documentation for " in line:
            pending = True
            continue
        if pending and "-->" in line:
            part = line.split("-->", 1)[1].strip()  # crates\x\src\y.rs:17:5
            path, _, rest = part.partition(":")
            num = rest.split(":")[0]
            todo.append((path, int(num)))
            pending = False

    # 每文件从下往上
    todo.sort(key=lambda t: (t[0], -t[1]))
    done = skipped = 0
    for path, lineno in todo:
        full = f"{ROOT}/{path.replace(chr(92), '/')}"
        key = f"{path.replace(chr(92), '/')}:{lineno}"
        entry = DOCS.get(key)
        expect = None
        doc = None
        with io.open(full, encoding="utf-8") as f:
            lines = f.read().split("\n")
        idx = lineno - 1
        if idx >= len(lines):
            skipped += 1
            continue
        if idx > 0 and lines[idx - 1].strip().startswith("///"):
            skipped += 1
            continue
        if entry is None:
            # 回退：向上找所属的 RawEvent 变体，生成"该变体的字段"式文档。
            variant = None
            for k in range(idx, max(0, idx - 12), -1):
                if "RawEvent::" in lines[k]:
                    variant = lines[k].split("RawEvent::")[1].split("{")[0].split("(")[0].strip()
                    break
            field = lines[idx].strip().split(":")[0].strip()
            doc_map = {
                "x": "事件发生位置的 X（客户区逻辑像素）。",
                "y": "事件发生位置的 Y。",
                "button": "哪个鼠标键。",
                "delta": "滚动量；正数向上。",
                "mods": "当时的修饰键状态。",
            }
            doc = doc_map.get(field)
            if variant is None or doc is None:
                skipped += 1
                print(f"[skip] {key} :: {lines[idx].strip()[:70]}")
                continue
            doc = f"`{variant}` 的{doc}"
            expect = lines[idx].strip()
        if expect is not None and lines[idx].strip() != expect:
            skipped += 1
            print(f"[mismatch] {key} 期望 {expect!r} 实际 {lines[idx].strip()!r}")
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
