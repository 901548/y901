#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""给 Y901 的 29 处 `#[allow(dead_code)]` 逐项标注类型与理由（v0.1.1 P2）。

分类只有三种：
  诊断 API   —— 生产代码不调用，但测试或环境变量开关依赖它
  测试 API   —— 仅测试使用（类型见证 / 未用常量）
  真死代码   —— 已确认零引用，直接删除
"""

import io

ROOT = "F:/Y901/"

# (相对路径, 行号) -> 分类与理由
MARKS = {
    # ---- backend-win32 ----
    ("crates/y901-backend-win32/src/backend/d3d11.rs", 1247):
        ("诊断 API", "com_type! 生成的接口别名只在内部与测试里具名使用；删除会削弱类型检查"),
    ("crates/y901-backend-win32/src/backend/d3d11.rs", 2775):
        ("诊断 API", "COM 泄漏计数，仅 `live_objects()` 测试与人工排查调用"),
    ("crates/y901-backend-win32/src/backend/font.rs", 560):
        ("诊断 API", "`ascent` 字段：自研引擎用它算基线，DirectWrite 路径由 `metrics()` 读"),
    ("crates/y901-backend-win32/src/backend/font.rs", 565):
        ("诊断 API", "`descent` 字段：同上，行高计算的一部分"),
    ("crates/y901-backend-win32/src/backend/font.rs", 656):
        ("诊断 API", "`glyph_count()`：图集占用诊断，`font_m3_atlas` 测试读取"),
    ("crates/y901-backend-win32/src/backend/font.rs", 704):
        ("诊断 API", "`has_dirty()`：后端据此跳过整张图集上传，测试直接断言"),
    ("crates/y901-backend-win32/src/backend/mod.rs", 9):
        ("诊断 API", "`pub mod d3d11`：内部模块对外可见仅为测试访问 `Gfx` 诊断接口"),
    ("crates/y901-backend-win32/src/backend/mod.rs", 312):
        ("诊断 API", "`instance_count()`：诊断，测试断言提交实例数"),
    ("crates/y901-backend-win32/src/backend/mod.rs", 317):
        ("诊断 API", "`live_objects()`：COM 泄漏诊断，测试断言\"用完全部释放\""),
    ("crates/y901-backend-win32/src/ime.rs", 22):
        ("诊断 API", "`caret` 字段由 `composing_text()` 返回，UI 层可选消费"),
    ("crates/y901-backend-win32/src/ime.rs", 63):
        ("诊断 API", "预编辑光标位置，供覆盖层绘制"),
    ("crates/y901-backend-win32/src/ime.rs", 132):
        ("诊断 API", "`set_composition_position()`：输入法候选窗跟随，当前未接线"),
    ("crates/y901-backend-win32/src/ime.rs", 233):
        ("测试 API", "`_unused`：类型见证，保证 `WPARAM`/`LRESULT` 仍被引用"),
    # ---- x11 ----
    ("crates/y901-backend-x11/src/window.rs", 57):
        ("诊断 API", "字体 id 由 `XSetFont` 设置后保留，供 `TextItem` 排字使用"),
    # ---- render ----
    ("crates/y901-render/src/font_engine/flatten.rs", 227):
        ("测试 API", "`_unused`：类型见证，保证 `Line` 仍被引用"),
    ("crates/y901-render/src/soft.rs", 772):
        ("测试 API", "`_unused`：类型见证，保证 `Edges` 仍被引用"),
    # ---- widgets ----
    ("crates/y901-widgets/src/scroll.rs", 336):
        ("诊断 API", "`target`：ScrollCommand 的目标容器名，仅记录（拆成独立组件前保留）"),
    ("crates/y901-widgets/src/scroll.rs", 469):
        ("测试 API", "`_pos`：类型见证，保证 `Pos` 仍被引用"),
}

# 真死代码：确认零引用后删除
DELETE = [
    ("crates/y901-widgets/src/slider.rs", 32),   # HIT_R
    ("crates/y901-widgets/src/slider.rs", 115),  # hits_thumb（仅测试调用 → 改成测试内联）
]

TEST_MARK = ("测试 API", "测试内的类型见证/未用常量，仅测试代码引用")


def apply(path: str, lineno: int, kind: str, reason: str) -> None:
    full = ROOT + path
    lines = io.open(full, encoding="utf-8").read().split("\n")
    idx = lineno - 1
    # 行号指向 allow 所在行（1-based）
    if idx >= len(lines) or "allow(dead_code)" not in lines[idx]:
        # 允许偏移 ±2（git grep 行号与段落可能差一行）
        for d in (-2, -1, 1, 2):
            if 0 <= idx + d < len(lines) and "allow(dead_code)" in lines[idx + d]:
                idx += d
                break
        else:
            print(f"[skip] {path}:{lineno} 找不到 allow")
            return
    indent = lines[idx][: len(lines[idx]) - len(lines[idx].lstrip())]
    lines.insert(idx, f"{indent}// {kind}：{reason}")
    io.open(full, "w", encoding="utf-8", newline="\n").write("\n".join(lines))


def main() -> int:
    done = 0
    for (path, lineno), (kind, reason) in MARKS.items():
        apply(path, lineno, kind, reason)
        done += 1
    print(f"标注 {done} 处")

    # 测试文件里的 9 处统一标注
    extra = [
        ("crates/y901-backend-win32/tests/input_events.rs", 15),
        ("crates/y901-backend-win32/tests/input_events.rs", 20),
        ("crates/y901-ui/tests/cursor_layers.rs", 366),
        ("crates/y901-ui/tests/revision.rs", 257),
        ("crates/y901-widgets/tests/file_drop.rs", 164),
        ("crates/y901-widgets/tests/focus_semantics.rs", 389),
        ("crates/y901-widgets/tests/focus_tab.rs", 116),
        ("crates/y901-widgets/tests/streaming_sticky_rows.rs", 329),
        ("crates/y901-widgets/tests/style_injection.rs", 293),
    ]
    for path, lineno in extra:
        apply(path, lineno, *TEST_MARK)
    print(f"测试文件标注 {len(extra)} 处")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
