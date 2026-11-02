#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""第 5 轮：y901-ui 剩余 154 项。**一次性**按本次 dump 的行号插入（每个文件从下往上），
避免跨轮漂移——第 3 轮的教训是行号会随插入变化，所以要么语义锚点，要么一轮做完。
"""

import io
import sys

ROOT = "F:/Y901"

# path:line -> doc
D = {
    # ================= color.rs =================
    "color.rs:6": "红通道（0..=1 线性空间，非 sRGB 编码值）。",
    "color.rs:7": "绿通道。",
    "color.rs:8": "蓝通道。",
    "color.rs:9": "不透明度；0 = 完全透明。渲染走预乘 alpha，混合时按覆盖率叠加。",
    "color.rs:13": "完全透明（也用于\「不绘制背景\「：`bg.a == 0` 时后端跳过背景填充）。",
    "color.rs:14": "纯黑。",
    "color.rs:15": "纯白；`TextStyle::default()` 的前景就是它（曾经误用透明色导致文字不可见）。",
    "color.rs:17": "由 0..=1 的三个通道构造，alpha 为 1。",
    "color.rs:21": "由 0..=1 的四个通道构造。",
    "color.rs:25": "由 0..=255 的字节构造，便于直接写设计稿上的颜色值。",
    "color.rs:75": "前景色。",
    "color.rs:76": "背景色；`TRANSPARENT` 表示不画背景（让下层透出来）。",
    "color.rs:77": "粗体。",
    "color.rs:78": "斜体。",
    "color.rs:79": "下划线。",
    "color.rs:95": "只设前景色，背景保持透明——大多数场景只需要这一项。",
    # ================= command.rs =================
    "command.rs:12": "组件与外界之间唯一的\「意图\「通道。框架只搬运、不解释语义。",
    "command.rs:15": "目标节点 id；`Node` 命令由树路由到该节点并转成 `UiEvent::Action`。",
    "command.rs:16": "动作名，由组件自己解释。",
    "command.rs:17": "动作参数。",
    "command.rs:20": "应用级命令：框架原样交给 `Host::command`，不理解 `name` 的含义。",
    # ================= event.rs =================
    "event.rs:13": "字符键。用 `Key::Char(' ')` 表示空格（没有单独的 Space 变体）。",
    "event.rs:14": "回车。",
    "event.rs:15": "Esc。",
    "event.rs:16": "退格。",
    "event.rs:17": "Tab；焦点遍历与组件自处理的分歧点。",
    "event.rs:18": "Delete。",
    "event.rs:19": "上方向键。",
    "event.rs:20": "下方向键。",
    "event.rs:21": "左方向键。",
    "event.rs:22": "右方向键。",
    "event.rs:23": "Home。",
    "event.rs:24": "End。",
    "event.rs:25": "PageUp。",
    "event.rs:26": "PageDown。",
    "event.rs:27": "功能键 F1..F24。",
    "event.rs:28": "无法识别的键；组件应忽略而不是当成 Char。",
    "event.rs:32": "修饰键状态。用结构体而不是位标志：组件读 `mods.ctrl` 比读掩码直观。",
    "event.rs:33": "Shift 是否按下。",
    "event.rs:34": "Ctrl 是否按下。",
    "event.rs:35": "Alt 是否按下。",
    "event.rs:39": "鼠标键。",
    "event.rs:40": "左键。",
    "event.rs:41": "右键；右键菜单与 `ContextMenu` 依赖它。",
    "event.rs:42": "中键（通常用于自动滚动）。",
    "event.rs:50": "按下的键值。",
    "event.rs:51": "当时的修饰键。",
    "event.rs:60": "拖入的文件绝对路径；框架只路由，不解释内容。",
    "event.rs:61": "松手位置（客户区坐标），用于命中目标控件。",
    "event.rs:65": "滚动量；正数向上。",
    "event.rs:66": "滚轮发生位置，决定滚哪个容器。",
    "event.rs:70": "鼠标按下。",
    "event.rs:71": "按下位置（客户区坐标）。",
    "event.rs:72": "哪个键。",
    "event.rs:82": "鼠标抬起。",
    "event.rs:83": "抬起位置。",
    "event.rs:84": "哪个键。",
    "event.rs:86": "指针移动；驱动 hover、拖拽与光标形状。",
    "event.rs:87": "当前位置。",
    "event.rs:98": "动作名（组件自定义）。",
    "event.rs:99": "动作参数。",
    "event.rs:123": "捕获指针：之后所有移动/抬起都强制发给该节点，直到 `Release`。"
        "为什么需要：拖拽滑块时指针会离开控件，没有捕获就会\"跟丢\"。",
    "event.rs:124": "释放指针捕获。",
    # ================= geometry.rs =================
    "geometry.rs:4": "应用层座标：（x, y）以客户区左上角为原点。为什么与 Size 分开：位置和尺寸的语义不同，混用会让布局代码难读。",
    "geometry.rs:5": "X 坐标（像素）。",
    "geometry.rs:6": "Y 坐标（像素）。",
    "geometry.rs:10": "构造一个点。",
    "geometry.rs:16": "尺寸。为什么不用（w, h）元组：尺寸会被四处传递（测量/布局/裁剪/缓存），命名字段比 .0/.1 安全。",
    "geometry.rs:17": "宽。",
    "geometry.rs:18": "高。",
    "geometry.rs:22": "零尺寸；作为「尚未布局」的初始值。",
    "geometry.rs:24": "构造一个尺寸。",
    "geometry.rs:28": "逐轴取较大值；容器求内容包围盒时用。",
    "geometry.rs:32": "逐轴取较小值；求两个约束的交集时用。",
    "geometry.rs:40": "矩形左上角 X。",
    "geometry.rs:41": "矩形左上角 Y。",
    "geometry.rs:42": "宽；可以为 0（空矩形），负值视为空。",
    "geometry.rs:43": "高。",
    "geometry.rs:47": "零矩形；`is_empty()` 为真，用于「尚未布局「。",
    "geometry.rs:54": "构造一个矩形。",
    "geometry.rs:58": "以原点为左上角、给定尺寸的矩形。",
    "geometry.rs:67": "右边界 = x + w（**不含**）；半开区间避免相邻区域重叠。",
    "geometry.rs:71": "下边界 = y + h（不含）。",
    "geometry.rs:75": "转成尺寸。",
    "geometry.rs:79": "点是否在矩形内（右/下边界不含）。命中测试与裁剪都用这一套语义。",
    "geometry.rs:99": "求交集；不相交时返回空矩形（而不是 panic）。",
    "geometry.rs:107": "是否为空（宽或高 ≤ 0）。",
    "geometry.rs:131": "按四边内缩；内缩后可能为空，调用方需自行判断。",
    "geometry.rs:144": "上边距。",
    "geometry.rs:145": "右边距。",
    "geometry.rs:146": "下边距。",
    "geometry.rs:147": "左边距。",
    "geometry.rs:151": "零边距。",
    "geometry.rs:158": "四边相等。",
    "geometry.rs:167": "水平/垂直各一个值。",
    "geometry.rs:176": "水平总占用（左+右）；计算子控件可用宽度时用。",
    "geometry.rs:180": "垂直总占用（上+下）。",
    # ================= json.rs =================
    "json.rs:8": "极简 JSON（只做序列化/反序列化，不做模式校验）。为什么不用 serde："
        "框架要求零重量级依赖，而 Agent 协议用到的类型屈指可数。",
    "json.rs:10": "null。",
    "json.rs:11": "布尔。",
    "json.rs:12": "数值（统一 f64；整数在协议层再收敛）。",
    "json.rs:13": "字符串。",
    "json.rs:14": "数组。",
    "json.rs:15": "对象；用有序 `Vec` 而不是 `HashMap` 是为了输出稳定（同输入必得同字节）。",
    "json.rs:19": "空对象；配合 `set` 链式构造。",
    "json.rs:35": "按 key 取值；不是对象或没有该键都返回 `None`。",
    "json.rs:42": "取字符串视图（不复制）。",
    "json.rs:49": "取布尔。",
    "json.rs:56": "取数值。",
    "json.rs:70": "解析一段 JSON 文本。`Err` 带位置信息，便于定位协议错误。",
    # ================= layout.rs =================
    "layout.rs:80": "建立空缓存。",
    "layout.rs:90": "统计信息（命中/未命中/节点数），用于评估缓存是否有效。",
    "layout.rs:116": "清空缓存；主题或字体变化后必须调用，否则旧度量会被复用。",
    "layout.rs:127": "某节点在本次布局里是否需要重新测量。",
    "layout.rs:481": "一维弹性容器：按主轴排列子控件，支持 padding、gap 与弹性分配。",
    "layout.rs:500": "按主轴方向创建。",
    "layout.rs:517": "水平排列（从左到右）。",
    "layout.rs:521": "垂直排列（从上到下）。",
    "layout.rs:525": "设置内边距；子控件可用空间相应减少。",
    "layout.rs:530": "设置子控件之间的间距。",
    "layout.rs:535": "设置背景色；`None` 时不画，避免不必要的填充。",
    "layout.rs:540": "设置自己在父容器里的尺寸约束。",
    "layout.rs:545": "追加子控件并返回自身（链式构造）。",
    "layout.rs:550": "追加子控件（就地修改，用于动态增删）。",
    "layout.rs:675": "创建一个单子节点容器（用于把任意控件塞进需要容器的位置）。",
    "layout.rs:687": "设置唯一子节点。",
    "layout.rs:692": "设置自己在父容器里的尺寸约束。",
    # ================= query.rs =================
    "query.rs:26": "结果条数上限；`None` = 不限。上限存在的理由是 Agent 一次拿几千条没有意义。",
    # ================= text.rs =================
    "text.rs:29": "建立空文本编辑缓冲。",
    "text.rs:46": "当前文本。",
    "text.rs:50": "字节长度（UTF-8）；分配缓冲时用它。",
    "text.rs:54": "字符数（不是字节数）；光标移动与选区都按字符计。",
    "text.rs:58": "是否为空。",
    "text.rs:83": "是否存在选区；Ctrl+C 据此决定\「复制还是把信号透给 shell\「。",
    "text.rs:118": "全选。",
    "text.rs:123": "清除选区（保留文本）。",
    # ================= theme.rs =================
    "theme.rs:14": "常态。",
    "theme.rs:15": "悬停态（鼠标在上）。",
    "theme.rs:16": "按下态。",
    "theme.rs:17": "键盘焦点态。",
    "theme.rs:18": "禁用态；同时应拒绝动作。",
    "theme.rs:43": "前景色（文字/图形）。",
    "theme.rs:44": "背景色。",
    "theme.rs:45": "边框色。",
    "theme.rs:46": "圆角半径（像素）。",
    "theme.rs:47": "内边距。",
    "theme.rs:48": "选区高亮色。",
    "theme.rs:49": "光标颜色。",
    "theme.rs:53": "只给前景与背景，其余取中性默认值——最常用的构造入口。",
    "theme.rs:87": "设置内边距并返回自身。",
    "theme.rs:92": "设置圆角半径并返回自身。",
    "theme.rs:106": "槽位数。新增角色必须落在这几个槽里，否则会被折叠到 panel。",
    "theme.rs:122": "槽位名；主题文件与调试输出用它做稳定标识。",
}


def main() -> int:
    # 从最新 dump 里重新取 (file, line)：由调用方生成 todo.tsv
    todo_path = f"{ROOT}/todo.tsv"
    rows = []
    with io.open(todo_path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line:
                continue
            path, lineno = line.split("\t")
            rows.append((path, int(lineno)))
    rows.sort(key=lambda t: (t[0], -t[1]))

    done = skipped = 0
    for path, lineno in rows:
        base = path.replace("\\", "/").rsplit("/", 1)[-1]
        full = f"{ROOT}/crates/y901-ui/src/{base}"
        with io.open(full, encoding="utf-8") as f:
            lines = f.read().split("\n")
        idx = lineno - 1
        if idx >= len(lines) or (idx > 0 and lines[idx - 1].strip().startswith("///")):
            skipped += 1
            continue
        doc = D.get(f"{base}:{lineno}")
        if doc is None:
            skipped += 1
            print(f"[skip] {base}:{lineno} :: {lines[idx].strip()[:60]}")
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
