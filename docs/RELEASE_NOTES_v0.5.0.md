# Y901 v0.5.0 发布说明

> 版本：`0.5.0` · 上一版：[`RELEASE_NOTES_v0.4.0.md`](RELEASE_NOTES_v0.4.0.md)（已发布）
>
> **状态：已准备好，尚未发布。** 自动化检查全绿；发布卡在三件外部条件上：
> crates.io 凭据（`cargo login`）、发布窗口、真机读屏器人工验收（见文末）。

## 为什么是 0.5.0 而不是 0.4.1

原计划是打补丁版 0.4.1。`cargo semver-checks` 对着 `v0.4.0` 基线跑出来的结论是**不能发 patch**：

```text
constructible_struct_adds_private_field: field Present.collector
crates/y901-backend-web/src/present.rs
```

`Present` 在 0.4.0 可以用结构体字面量构造（`pub canvas` + `pub ctx`），而本轮给它加了私有字段
`collector` 存分阶段统计 —— 外部写 `Present { canvas, ctx }` 会编译失败。

按本仓库的规矩（**0.x 的破坏性变更升 minor**，0.4.0 就是这么来的：`WasmState` 增 `pub` 字段），
所以是 **0.5.0**。没有为了"保住 patch 号"去把字段搬到别处 —— 那只是把破坏换个位置
（`WasmState`、`WasmWindow` 同样可字面量构造）。

## 用户能感觉到的修复（6 个真 bug）

| # | 症状 | 根因 |
|---|---|---|
| 1 | 终端里 `ESC[K` / `ESC[J` 擦到中文一半，**画面上少半个字**；之后往这行写字还会吃掉相邻内容 | 擦除逐格填空白，把宽字符的"头/尾"劈开；现在按整字向外扩 |
| 2 | 改窗口宽度（`reflow`）时偶发 **panic**，release 下静默错位到错误的行首 | 裁剪历史时假设"同一逻辑行的偏移单调"，`soft_wrap` 让该假设不成立；改为连续前缀 + 累加重算 |
| 3 | 用读屏器点"删除第 2 行"，**删掉的是第 3 行** | `NodeId` 是遍历序号，删行后整体前移，而读屏器动作是异步到达的 |
| 4 | 流式输出（日志、进度）**一顿一顿地跳** | `ScrollView` 的粘底在"摆完内容之后"才改偏移，同一帧里位置与偏移不一致 |
| 5 | `Table` 定宽列超宽时，**最后一列画在可视区外**，表头点击指向看不见的地方 | "超宽按比例压缩"的判据用了 `rest`（定宽列超宽时恰为 0），压缩被跳过 |
| 6 | 后台线程每来一条数据就**整窗重绘 + 整帧上传** | `drain_external_invalidations` 把单节点标脏升级成整窗（`whole_window` 是死变量） |

另外两个与设备/平台相关：

| # | 症状 | 根因 |
|---|---|---|
| 7 | 设备丢失重建后 WARP 上 `STATUS_HEAP_CORRUPTION`（真实 GPU 上悄悄踩坏驱动内存） | 实例缓冲容量记在 `Gfx`、缓冲在 `Pipeline`，重建后二者不同步 → 写映射区尾部 |
| 8 | Win32 应用**无法否决关闭**；IME 候选窗**从不跟随光标** | `WM_CLOSE` 落到 `DefWindowProcW` 直接销毁窗口；`CandidateForm` 用错 `CFS_*` 位与坐标系 |

## 新增能力

- **文本编辑三件必备操作**（此前都没有）：`Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` 撤销重做
  （连续打字 700ms 合并为一步，200 步 + 4 MiB 双上限，撤销还原光标）、
  `Ctrl+←/→` 按词移动（`Shift` 扩选）、**双击选词**。
- **`TextArea` 的 Tab 缩进**：`tab_indents(true)` 开启后 Tab 补到下一个制表位（插空格，
  不是 `\t`）、`Shift+Tab` 反缩进。默认**不开启**（Tab 仍归焦点遍历）。
  框架同时保证 **`Ctrl+Tab` 永远切焦点** —— 否则这就是键盘陷阱。
- **无障碍**：`NodeId` 改为 `NodeInfo.id` 的稳定哈希，屏读器缓存与焦点记忆跨帧有效；
  浮层（菜单/下拉）现在也在语义树里，且矩形与命中测试一致。

## 性能

| 项目 | 改前 | 改后 |
|---|---|---|
| 光标移动（20 万字符，1000 次右移） | 41.2 s | **327 µs** |
| 图片 mip 生成（4 MiB 图，累计分配） | 3.31 × L0 | **1.313 × L0**（= 1+1/4+1/16，理论最优） |
| 后台单节点标脏 | 整窗重绘 | 只重绘该节点 + 一帧重排 |

## 工程与证据

- 新增 **15 套属性测试**（确定性伪随机，不引依赖）：终端网格、图集打包、脏区覆盖、
  Flex 布局、光标字素簇、折行、Bidi 视觉序、X11 请求编码、ListView/TreeView 虚拟化、
  选区/剪贴板、事件循环、文本输入状态、Tab 缩进、图片 mip 分配、`wrap` 不变量。
  它们抓到上面 6 个 bug 中的 5 个。
- 计数分配器（`#[global_allocator]`）用于在没有 GPU 的 CI 里守住"上传不再多拷贝"。
- 仓库统一 `eol=lf`（此前 `core.autocrlf=true` 让每次 `git add` 都刷 CRLF 警告，
  且下次 checkout 会真的改写工作区）。
- 两处门禁脚本自身的坑：`gate.ps1` / `sync-doc-numbers.ps1` 用 `Get-Content` 按 ANSI 读
  `test-baseline.txt`，中文注释会把末尾数字行粘进注释 → 改为显式 UTF-8 读取。

## 已知边界（不假装解决）

- **设备丢失恢复路径没有自动化覆盖**：触发它需要拔显卡/驱动更新。第 7 条靠"WARP 上恰好能稳定
  复现"定位，结论已写成结构守卫，但"真实 GPU 上设备真的丢了会怎样"仍是人工范围。
- **屏幕阅读器实际播报**：自动化只证明"语义树里有正确的东西"，念得对不对要真人
  （[`A11Y_ACCEPTANCE.md`](A11Y_ACCEPTANCE.md) 10 分钟清单，本轮新增 17b"删中间一行再点删除"）。
- **IME 候选窗**：只验到表单参数正确，真实输入法把候选框画在光标旁仍需人工看。
- **`box_levels_rgba` 的像素等价性**：零拷贝后累计分配降到理论最优、像素哈希不变，
  但没有逐字节对拍旧实现的读回路径（`box_levels_rgba` 是私有的，没有公开读回接口）。

## 发布步骤（维护者执行）

```powershell
# 1. 凭据（一次性）
cargo login            # 或设 CARGO_REGISTRY_TOKEN

# 2. 门禁与就绪
powershell -NoProfile -File scripts/gate.ps1                # 必须 GATE OK
powershell -NoProfile -File scripts/check-release-readiness.ps1
powershell -NoProfile -File scripts/check-consumer.ps1 -Version 0.5.0

# 3. 按依赖序发布 12 个 crate
powershell -NoProfile -File scripts/publish-crates.ps1

# 4. 打 tag 并推送（发布内容必须与 tag 一致）
git tag -a v0.5.0 -m "Y901 v0.5.0"
git push origin v0.5.0
```

> `git push` 到 github 本机常被 reset：用
> `git -c http.sslBackend=openssl -c http.sslVersion=tlsv1.2 push origin main`，失败重试。