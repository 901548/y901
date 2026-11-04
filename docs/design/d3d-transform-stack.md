# D3D 变换栈：下一轮的前置约束与实施顺序

> 状态：**已实现（v0.1.5）**；验证见 `crates/y901-backend-win32/tests/d3d_text_scaling.rs`。
> 下面的前置约束保留为设计过程记录（**顺序不能反**）。

## 0. 为什么需要它

全仓库只有 3 处 `push_transform` 调用，而 **`y901-backend-win32` 里出现 0 次**：

| 调用点 | 用途 |
|---|---|
| `y901-ui/src/tree.rs:1741` | **Popup 打开动画的缩放**（`scale = 0.8 + 0.2·progress`） |
| `y901-widgets/src/tabs.rs:332` | Tabs 内容切换的平移（`scale = 1.0`） |
| `y901-ui/src/widget.rs:402` | trait 默认**空实现** |

`RenderCtx::push_transform` 的默认实现是空操作，**只有 `SoftBackend` 覆盖了它**。
于是在 Windows/D3D 上：

- **Popup 打开动画完全不缩放**（面板、边框、阴影纹丝不动，只有最后那层 `(1-progress)` 透明度遮罩在变）；
- **文本缩放永远不触发**（`glyph_at_scale` 只能拿到 `scale = 1.0`）。

这是**既存缺陷**（v0.1.3 之前就存在），不是 v0.1.4 引入的；所有 Popup 动画的像素断言都跑在
SoftBackend 上，所以从未暴露。

## 1. 前置约束：**先证明"不变"，再证明"变了"**

如果把变换逻辑和恒等性一起提交，哈希变了就分不清是**哪一类**问题：

- 恒等性没守住（无变换时浮点往返引入了误差），还是
- 变换逻辑本身改变了正常渲染。

所以必须分两步，且顺序不可交换。

### 第 1 步：空变换栈 + 恒等性验证（**必须先做**）

1. 给 `D3dBackend` 加 `transforms: Vec<(Pos, f32)>`，初始为空；
2. 在 5 条出图路径（`fill_rect` / `fill_round_rect` / `fill_gradient_rect` / `push_glyph` / `draw_image`）
   各插入一次 `let pos = self.map_pos(pos); let size = self.map_size(size);`；
3. **栈为空时 `map_pos` / `map_size` 必须原样返回，不做任何算术**：

```rust
fn map_pos(&self, p: Pos) -> Pos {
    if self.transforms.is_empty() {
        return p;            // ← 关键：不进任何浮点运算，保证逐位恒等
    }
    let mut x = p.x;
    let mut y = p.y;
    for (o, s) in self.transforms.iter().rev() {
        x = o.x + (x - o.x) * s;
        y = o.y + (y - o.y) * s;
    }
    Pos::new(x, y)
}
```

4. 跑 `pwsh -File scripts/gate.ps1`；
5. **判据：像素哈希仍是 `0xF38B9C1AB8A877E2` / 382×173**。

**如果哈希变了** → 停下来修 `map_pos`（多半是 `* 1.0` 后又 `round`，或走了 f32 往返），
**不要继续接入真正的变换逻辑**。

### 第 2 步：实现 `push_transform` / `pop_transform` + 文本 LOD

恒等性通过后再做：

1. `push_transform(origin, scale)` 压栈；`pop_transform` 弹栈；
2. `push_glyph` 里：`let sel = self.text.glyph_at_scale(ch, self.current_scale())?;`
   —— `uv_rect` 指向 `sel.slot`，目标尺寸用 `sel.w × scale`，bearing 用 `sel.slot.bearing_*`；
3. 像素断言（用 `take_snapshot` 回读）：
   - `s = 1.0` 与改造前逐像素一致（**变异对照**：改一个像素应被发现）；
   - `s = 0.5` 墨量比 ≈ 0.25 ±30%（与 SoftBackend 同一口径）；
   - 与 SoftBackend 的 MAD ≤ 0.05（GPU 浮点 vs CPU 定点，允许小差异）。

## 2. 要动的出图路径与代价

| 方法 | 改动 |
|---|---|
| `fill_rect` | 位置 + 尺寸过变换 |
| `fill_round_rect` | 同上 + **圆角半径乘 scale** |
| `fill_gradient_rect` | 同上 |
| `push_glyph` | 位置 + 字号 + **UV 换级** |
| `draw_image` | 位置 + 尺寸 |

**共同点**：都先算好 Instance 的 `pos`/`size` 再 push，所以变换可以集中在 `map_pos` / `map_size`
两个辅助函数；但每条路径的**调用点都要改**。

## 3. 工作量估算

| 部分 | 时间 |
|---|---|
| `map_pos` / `map_size` + 恒等性验证 | 30 分钟 |
| 5 条出图路径接入 | 60 分钟 |
| 文本 LOD（`glyph_at_scale` + UV 换级） | 30 分钟 |
| D3D 像素断言（`s=1` 不变 + `s=0.5` 墨量） | 45 分钟 |
| 哈希基线重建（若恒等性没守住才会用到） | 30 分钟 |
| **合计** | **≈ 3 小时** |

**主要风险**：恒等性。无变换时任何浮点往返都会让哈希漂移，
而哈希门禁会把这种漂移当成"渲染回归"。

## 4. 完成后的连带收益

- Popup 打开动画在 Windows GPU 上**终于会缩放**（既存缺陷修复）；
- Tabs 的平移动画在 GPU 上生效；
- 两个后端的文本缩放行为一致，`RELEASE_NOTES_v0.1.4` 里那条已知限制可以删除。
