# D3D11 图片纹理路径：现状调查与实施方案

> 状态：**未实施**。本文记录调查结论与实施步骤，供下一轮直接接手。
> 结论先行：这不是"加一个纹理"的活，而是**渲染批次的拆分问题**。

## 1. 已经有的（不需要重做）

| 能力 | 位置 | 说明 |
|---|---|---|
| `Instance.uv_rect: [f32; 4]` | `d3d11.rs:65` | 实例里**已经有 UV 字段**，VS 会按顶点插值成 `output.uv` |
| 采样纹理的像素着色器 | `ps_main`（`d3d11.rs:1169`） | 字形分支已经在 `atlas_texture.Sample(...)`，采样通路现成 |
| 采样器 | `Gfx.atlas.sampler` | 线性过滤 + Clamp，与 SoftBackend 的双线性语义一致 |
| 纹理创建 / SRV | `create_texture_2d`、`create_texture_2d_empty`、`create_shader_resource_view` | 可直接复用 |
| 上传 | `update_subresource`、`update_subresource_region` | 可直接复用 |
| 像素回读 | `read_back_frame()`、`Window::take_snapshot()` | 测试可用，`font_3c_pixels` 已在用 |

**`params` 的第 4 个分量是空闲的**：`params = (radius, softness, stroke_width, 0)`（见 `d3d11.rs:1174`），
所以可以用 `params.w` 当"这是图片"的标志位，不必新增实例字段、不必改顶点布局。

## 2. 真正的障碍：整帧是**一次** instanced draw

UI 实例全部累加进同一个 buffer，然后：

```rust
context.set_ps_texture(&atlas.srv, &atlas.sampler);  // 只绑定了图集
context.draw_instances(count);                        // 一次画完
```

一张 SRV 绑定贯穿全帧。图片要用**另一张**纹理，就必须把这一帧拆成若干段：

```
[图集实例段] → 绑定图集 → draw
[图片A 实例段] → 绑定图A → draw
[图片B 实例段] → 绑定图B → draw
[图集实例段] → 重新绑定图集 → draw
```

所以关键改动是**批次拆分**，纹理管理反而是小事。

## 3. 实施步骤（建议顺序）

1. **HLSL**（`SHADER_SOURCE`，约 6 行）
   ```hlsl
   Texture2D image_texture : register(t1);
   // ps_main 开头：
   if (input.params.w > 0.5)
   {
       float4 tex = image_texture.Sample(atlas_sampler, input.uv);
       float a = input.color.a * tex.a;
       return float4(input.color.rgb * tex.rgb * a, a);   // 混合状态是预乘 alpha
   }
   ```
2. **纹理表**：`textures: HashMap<TextureId, Entry { _texture: Texture, srv: ShaderResourceView, w: u32, h: u32 }>`
   + `next_texture: u64`。`upload_texture` 用 `create_texture_2d(desc, Some(&SubresourceData{ p_sys_mem: rgba, sys_mem_pitch: w*4 }))`。
3. **批次拆分**：把"累加实例 + 一次 draw"改成 `Vec<Batch>`，`Batch { srv: BatchSrv::Atlas | Texture(TextureId), instances: Vec<Instance> }`。
   相邻同 SRV 的实例合并；`draw_image` 需要把当前批次切段。
4. **`draw_image` 实现**（`impl RenderCtx for Gfx`）：
   - `ImageStyle::layout(rect, tw, th)` 得到 `(dst, src)` —— **复用纯函数，不要重推 fit**；
   - `uv_rect = [src.x/tw, src.y/th, (src.x+src.w)/tw, (src.y+src.h)/th]`；
   - `params.w = 2.0`；`color = tint.unwrap_or(白)`。
5. **像素测试**（`crates/y901-backend-win32/tests/d3d_image.rs`，模板照抄 `font_3c_pixels.rs`）：
   上传 2×2 红/绿/蓝/白 → 画到 20×20 → `take_snapshot()` → 四象限主色断言；
   再跑 Cover/Contain/Tint 三个用例；与 SoftBackend 的结果比对，容差 ±2。

## 4. 已知风险

- **行宽对齐**：`R8G8B8A8_UNORM` 的 row pitch 天然 4 字节对齐（`w*4`），不需要 padding。
- **纹理回收**：`RenderCtx` 没有 `release_texture`，`Widget` 也没有 unmount 钩子。
  第一版建议**不回收**（图片数量少，一张 1024² = 4MB），记入已知限制；要回收得先给
  `RenderCtx` 加 `release_texture(TextureId)` 并在控件 Drop 时拿到 ctx —— 后者不存在。
- **性能**：每张图片一次 draw call。图片不多时可接受；要优化就上纹理图集（P2）。

## 5. 为什么这一轮停在这里

改批次拆分要动全帧渲染路径，而这条路径目前 646 个测试全绿、UI 正常显示。
在没有足够时间完成"改动 + 像素验证"的情况下，半接线会让**所有窗口的渲染**处于未知状态，
风险远大于收益。宁可不做，也不把可用的东西改坏。
