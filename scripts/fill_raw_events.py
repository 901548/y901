#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""给 `RawEvent` 的字段补文档：按"所属变体 + 字段名"生成，不依赖行号。

为什么单独一个脚本：这些字段重复度极高（4 个 `x: f32,`、3 个 `mods: Modifiers,`），
行号会随插入漂移、片段又不足以区分，只有"当前处在哪个变体里"才是稳定信息。
"""

import io
import re
import sys

PATH = "F:/Y901/crates/y901-runtime/src/lib.rs"

FIELD_DOC = {
    "x": "位置 X（客户区逻辑像素）。",
    "y": "位置 Y。",
    "button": "哪个鼠标键。",
    "delta": "滚动量；正数向上。",
    "mods": "当时的修饰键状态。",
    "key": "键值。",
    "c": "字符。",
    "w": "新的客户区宽度。",
    "h": "新的客户区高度。",
    "text": "组字中间串或最终串。",
    "composition": "`true` = 预编辑（只画不提交）；`false` = 已提交。",
    "paths": "拖入的文件绝对路径。",
}


def main() -> int:
    with io.open(PATH, encoding="utf-8") as f:
        lines = f.read().split("\n")

    start = next(i for i, l in enumerate(lines) if l.strip().startswith("pub enum RawEvent"))
    # 找到枚举结束（顶格 "}"）
    end = next(i for i in range(start + 1, len(lines)) if lines[i].startswith("}"))

    out = lines[: start + 1]
    variant = None
    depth = 1
    inserted = 0
    for i in range(start + 1, end):
        line = lines[i]
        stripped = line.strip()

        m = re.match(r"([A-Z][A-Za-z]*)", stripped)
        if m and stripped.endswith("{") and not stripped.startswith("//"):
            variant = m.group(1)

        # 字段行：`name: Type,` 且没有紧邻的 `///`
        fm = re.match(r"^([a-z_]+):\s", stripped)
        if fm and depth == 2 and variant:
            field = fm.group(1)
            prev = out[-1].strip() if out else ""
            if not prev.startswith("///") and field in FIELD_DOC:
                indent = line[: len(line) - len(line.lstrip())]
                out.append(f"{indent}/// `{variant}` 的{FIELD_DOC[field]}")
                inserted += 1

        if stripped.endswith("{") or stripped.endswith("("):
            depth += 1
        if stripped.startswith("}") or stripped.endswith("},") or stripped.endswith("),"):
            depth = max(1, depth - 1)
        out.append(line)

    out.extend(lines[end:])
    with io.open(PATH, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))
    print(f"插入 {inserted} 处")
    return 0


if __name__ == "__main__":
    sys.exit(main())
