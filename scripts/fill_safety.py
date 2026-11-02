#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""给 d3d11.rs / font.rs 剩余 51 个 `unsafe fn` 补 `# Safety`。

分类依据是**真实的契约形状**，不是函数名猜的：
- COM 生命周期（release / from_raw / bytes）
- 设备工厂（create_*）：设备存活 + 输入结构存活 + 返回值所有权转移
- 上下文状态（set_*/clear/draw/copy/map/update）：immediate context 单线程 + 资源同设备
- 内部编排（compile_shader / create_pipeline / draw_scene / blit_cached …）：仅在窗口线程 + Gfx 已初始化
"""

import io
import re

D3D11 = "F:/Y901/crates/y901-backend-win32/src/backend/d3d11.rs"
FONT = "F:/Y901/crates/y901-backend-win32/src/backend/font.rs"

COM_LIFECYCLE = {
    "release": (
        "`self.ptr` 要么为空、要么是一个**仍持有引用**的合法 COM 接口指针；本函数按 COM 规则调用\n"
        "`Release` 并把指针置空，因此同一对象不得被并发释放（`&mut self` 已保证串行）。"
    ),
    "from_raw": (
        "`raw` 必须是某次 `Create*`/`Query*` 返回、且**所有权尚未转交他人**的接口指针；\n"
        "本函数接管所有权（`Drop` 时释放）。传 null 返回 `None`，不会 panic。"
    ),
    "bytes": (
        "`self` 必须是 `D3DCompile` 成功返回的 blob；blob 存活期间其缓冲只读有效，\n"
        "返回的切片不得越过 blob 的生命周期使用。"
    ),
}

FACTORY = (
    "`self` 必须是**本线程上未释放**的 ID3D11Device（或 DXGI 工厂）；\n"
    "所有出参结构/描述指针由调用方提供且在调用期间保持有效、非空、按 D3D11 要求对齐；\n"
    "返回的 COM 对象所有权转交调用方（`Drop` 时 Release），失败时返回 `Err` 且不产生裸指针。"
)

CONTEXT = (
    "`self` 必须是**本线程独占**的 immediate context（D3D11 context 非线程安全，不得跨线程调用）；\n"
    "传入的资源必须由同一设备创建且尚未释放；`Map` 得到的指针只在配对的 `Unmap` 之前有效。"
)

INTERNAL = (
    "只在**拥有该 HWND 的线程**上、且 `Gfx` 已完成初始化后调用：\n"
    "内部直接解引用 vtable 与裸指针（交换链/设备/上下文），外部无法伪造这些前置条件。"
)

D3D11_MAP = {
    **{k: FACTORY for k in [
        "create_buffer", "create_render_target", "create_input_layout", "create_vertex_shader",
        "create_pixel_shader", "create_blend_state", "create_rasterizer_state",
        "create_texture_2d", "create_texture_2d_empty", "create_shader_resource_view",
        "create_sampler_state",
    ]},
    **{k: CONTEXT for k in [
        "set_render_target", "clear", "set_viewport", "update_constant_buffer",
        "update_subresource", "update_subresource_region", "unmap_texture", "set_pipeline",
        "draw_instances", "set_ps_image_texture", "set_ps_texture", "copy_resource",
        "map_read", "set_ps_shader", "back_buffer", "resize_buffers",
    ]},
    **{k: INTERNAL for k in [
        "compile_shader", "create_pipeline", "draw_scene", "blit_cached", "create_atlas",
        "read_back_frame", "ensure_cached", "create_device_and_swap_chain",
    ]},
    **COM_LIFECYCLE,
}

FONT_COM = (
    "`self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），\n"
    "且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）\n"
    "由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。"
)
FONT_RAW = (
    "传入/返回的裸指针必须来自同一个未释放的 DirectWrite 对象；\n"
    "本函数不做引用计数，调用方靠外层 `com_type!` 的所有权保证存活期。"
)
FONT_MAP = {
    **{k: FONT_COM for k in [
        "system_font_collection", "glyph_run_analysis", "find_family", "first_font",
        "create_face", "metrics", "design_metrics", "glyph_index", "alpha_bounds", "coverage",
    ]},
    "rasterize": (
        "`self` 必须是未释放的字形分析器；`advance_pen`/`origin` 必须指向调用方持有的有效内存，\n"
        "`outline` 的容量必须足以容纳 `GetGlyphRunOutline` 写出的轮廓点数（本文件按 4096 点预留）。"
    ),
    **COM_LIFECYCLE,
}


def insert(path: str, mapping: dict[str, str]) -> int:
    lines = io.open(path, encoding="utf-8").read().split("\n")
    out, done = [], 0
    i = 0
    while i < len(lines):
        ln = lines[i]
        m = re.search(r"unsafe fn (\w+)", ln)
        if m and not ln.strip().startswith("//"):
            name = m.group(1)
            # 是否已有 # Safety（向上扫连续的 /// 与属性）
            j = len(out) - 1
            has = False
            while j >= 0 and (out[j].strip().startswith("///") or out[j].strip().startswith("#[")):
                if "# Safety" in out[j]:
                    has = True
                    break
                j -= 1
            if not has and name in mapping:
                indent = ln[: len(ln) - len(ln.lstrip())]
                block = [f"{indent}/// # Safety", f"{indent}///"]
                for part in mapping[name].split("\n"):
                    block.append(f"{indent}/// {part}" if part else f"{indent}///")
                out.extend(block)
                done += 1
        out.append(ln)
        i += 1
    io.open(path, "w", encoding="utf-8", newline="\n").write("\n".join(out))
    return done


print("d3d11:", insert(D3D11, D3D11_MAP))
print("font :", insert(FONT, FONT_MAP))
