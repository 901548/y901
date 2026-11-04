# v0.1.4 发布说明

## 修复

- **clippy 92 → 0**（真实数字，此前报告的 "0/0" 是测量伪影）
  - 82 × `clippy::duplicate_underscore_argument`：只把下划线参数 `_c` 改名为 `_cache`。
    先尝试过 `c → constraints`，但 `c` 在函数体里有 4 种引用形态
    （`c.constrain(..)`、`c.max.w`、`measure(m, c, cache)`、`c.max.h`），每修一处又冒一处；
    `_c` 按约定函数体内不引用 → 56 个文件零函数体改动。
  - 10 × `clippy::to_string_in_format_args`：删掉**格式参数位置**多余的 `.to_string()`。
- **为什么之前没发现**：clippy 检查从未进入门禁脚本（每次手打），
  且手打的正则 `^warning: [a-z]` 匹配不到以反引号开头的消息。两层叠加 → 92 个警告长期不可见。

## 新增

- **`scripts/gate.ps1`**：一条命令跑完 MSRV（rustc ≥ 1.92）+ `cargo test` + clippy +
  像素哈希 + `missing_docs`，全部通过才输出 `GATE OK`。
  clippy 计数改用 `--message-format=json` + 统计 `"level":"warning"`，**不再用文本正则**。
  支持 `-SkipTest` 用于本地快速循环。
- **文本缩放（mipmap LOD）**，CPU 后端完整可用：
  - 字形按缩放比选级（1×/1/2×/1/4× 三级），采样 O(1)
  - 惰性生成（首次缩小时 ~14µs/字形），保住了 "2048² 装得下 1200 CJK × 4 相位" 的容量承诺
  - 图集占用保护：`occupancy > 0.8` 时跳过 L2
  - 质量实测（8×8 超采样真值）：`'A'` MAD **0.0095**、`'l'` **0.0275**，均 ≤ 0.05，断言已进回归

## 质量修正（都由 MAD 断言逼出来的）

| 修正 | 效果 |
|---|---|
| LOD 加级门槛 `>= 1` → `>= 2` | `'A'` 0.1987 → 0.0899（原本把 L2 放大到 L1 尺寸采样） |
| `MipSlot` 存**每级自己的 bearing** | `'A'` 0.0899 → 0.0172（原来混用 L0 的 bearing，偏 ~1px） |
| 超采样 4×4 → 8×8 | 位图下界 `'l'` 0.0735 → 0.0294（4×4 的量化步长就是那个"质量差距"） |
| 墨迹断言改用**墨量**（覆盖率积分） | 像素个数比 0.339（超限）→ 墨量比 **0.246**（理论 0.25） |

## 已知限制（本版本新增记录）

- **文本缩放仅 CPU 后端**：SoftBackend 支持 mipmap LOD 文本缩放，**D3D11 后端因未实现变换栈暂不支持**。
  D3D 上的 `push_transform` 是 trait 默认空实现，导致**文本缩放与 Popup 打开动画的缩放都不生效**
  （Windows GPU 路径上 Popup 只有最后那层透明度遮罩在变）。待 v0.1.5 修复。

## 状态

- 测试：**798**（`cargo test --workspace`）
- clippy：**0 警告 / 0 错误**（`--message-format=json` 口径）
- `missing_docs`：**0**（11 个 crate `deny`）
- 像素哈希：`0xF38B9C1AB8A877E2` / 客户区 382×173
- 门禁：`pwsh -File scripts/gate.ps1` → `GATE OK`
- tags：v0.1.0, v0.1.1, v0.1.2, v0.1.3, **v0.1.4**
