# v0.1.5 发布说明

## 新增

- **D3D11 变换栈真正可用**
  `push_transform` / `pop_transform` 此前是 trait 的空默认实现，D3D 后端**没有变换栈**。
  现在：`map_pos`（逐层累乘，内层先应用）、`map_size`、`map_rect`、`current_scale`，
  五条出图路径（`fill_rect` / `fill_round_rect` / `fill_gradient_rect_dir` / `push_glyph` / `draw_image`）全部过变换。
- **D3D11 文本缩放（mipmap LOD）**
  `push_glyph` 改用 `ScaledGlyph`：UV 指向所选级的图集矩形、尺寸按 `w × scale`、
  **bearing 用所选级自己的**（每级都在自己字号下光栅化，bearing 相差 0.5~1px）。

## 修复

- **Popup 打开动画在 Windows GPU 上不缩放**（既存缺陷，v0.1.3 及之前一直存在）
  由于 `push_transform` 是空操作，动画里只有 `(1 - progress)` 的透明度遮罩在变，面板本体不动。

## 验证（已进回归，不再 `#[ignore]`）

`crates/y901-backend-win32/tests/d3d_text_scaling.rs`：

| 断言 | 实测 |
|---|---|
| `s = 1.0` 与"不压栈"逐像素一致 | **0 字节不同**（hash `0xb1b2b6f2ed822050`），含**变异对照** |
| `s = 0.5` 墨量比 / 像素个数比 | **0.326 / 0.317**（同场景 CPU：0.246 / 0.339） |
| D3D vs SoftBackend MAD | **0.0012**（阈值 0.05，40 倍余量） |

**两个口径一起测的理由**：墨量（覆盖率积分）低估抗锯齿边缘、像素个数高估它，
单一口径总有一边贴边；CPU 侧历史数据（0.246 / 0.339）正好是这对夹逼的两端。

**回读尺寸**：`240×120` 的窗口得到 **218×64** 客户区（含边框与标题栏），断言按实际尺寸统计。

## 安全网

变换栈接入时**先证恒等、再证变换**：

1. 空栈 + 五条路径接入 → 像素哈希 `0xF4C92C36969FEC48` **逐位未变**；
2. 再实现 `push/pop` → 哈希仍不变（探针场景无动画）；
3. 最后接文本 LOD → 三条断言全过。

若把 1 和 2 合成一步，哈希一旦变化就分不清是"恒等性失守"还是"变换改变了正常渲染"。

## 状态

- 测试：**801**（798 + 3 条 D3D 探针）
- clippy：**0 警告 / 0 错误**
- `missing_docs`：**0**
- 像素哈希：`0xF4C92C36969FEC48` / 378×164
- 门禁：`pwsh -File scripts/gate.ps1` → `GATE OK`
- tags：v0.1.0 … **v0.1.5**
