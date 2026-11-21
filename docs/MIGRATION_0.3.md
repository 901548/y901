# 从 0.2.x 迁移到 0.3.0

> 状态：**进行中**。0.3.0 尚未发布；本文随 0.3 系列逐批补齐（对应 `docs/ROADMAP.md` 的 S1–S4）。
> 目标：把"破坏性变更"集中在 0.3.0 一次发生，而不是散落在 patch 版本里。

## 已发生的破坏性变更

### 1. `Semantics` 增加了无障碍字段，并标记 `#[non_exhaustive]`

新增字段：`description`、`expanded`、`checked`、`selected`、`level`、`value_range`、
`value_step`、`labelled_by`、`described_by`、`live`。

**为什么**：屏幕阅读器需要"名称 / 描述 / 展开 / 勾选 / 选中 / 层级 / 值域"这些独立槽位，
把它们塞进 `props` 会让 a11y 桥只能靠字符串猜语义。

**怎么改**：用 `Semantics::default()` + builder 方法（`.checked(true)`、`.value_range(0.0, 100.0)` …），
不要用结构体字面量。`#[non_exhaustive]` 会直接拦住字面量构造，把问题暴露在编译期而不是运行时。

> 附带影响：`hash_semantics` 已把这些字段纳入指纹，因此"复选框从选中变未选中"这类变化
> 现在会真正触发语义失效（以前不会，屏幕阅读器读不到更新）。

### 2. `TableColumn` 增加 `sortable` 字段，并标记 `#[non_exhaustive]`

`TableColumn::fixed(..)` / `TableColumn::flex(..)` 的返回值不受影响；
用结构体字面量构造列的代码需要改为 builder，需要禁止排序时加 `.not_sortable()`。

### 2b. `Options` 标记 `#[non_exhaustive]`

窗口选项只会增加（新后端能力、新配置项），不设这个属性则每加一个字段都是破坏性变更。

**怎么改**：用 `..Default::default()` 兜底：

```rust
let options = Options {
    title: "My app".into(),
    width: 640,
    height: 480,
    ..Default::default()
};
```

全字段都显式写时 Rust 也接受（此时 `..Default::default()` 可省，
但 clippy 的 `needless_update` 会提示；建议干脆保留），
仓库内 46 处字面量已按这个约定统一。

### 3. `Table` 换成虚拟滚动实现（行为变化）

- 表头**固定**在顶部，行区域独立滚动（滚轮 / 拖拽滚动条 / 键盘都支持）；
- 高度语义：小表仍按内容高度撑开；内容高于可用高度时**裁到可用高度**并出现滚动条
  （0.2.x 是"全部画出来，溢出交给父容器裁剪"）；
- 行高统一（虚拟滚动前提）；可变行高仍未支持；
- 新增 `set_rows()`、`select()`、`sort_by()`、`sort_state()`、`sort_command()`、`bar_style()`；
- **选中行语义变了**：`selected_index()` 返回**数据下标**，排序不会让选中行"变成另一行"
  （0.2.x 没有排序，所以这一点对老代码等价）。

### 4. 新增组件

- `NumberInput`：数值输入（步进 / 范围钳制 / 小数位 / Esc 放弃）。
- `MenuBar` / `MenuEntry`：菜单栏（弹出 `Menu` 浮层）。
- `Table::sort_by` 与 `SortOrder`（`widgets::SortOrder`）。

## 平台与文档

- 平台分层见 [`PLATFORMS.md`](PLATFORMS.md)：Windows = T1、Linux/X11 = T2、Web = T3（演示级），
  macOS 与 Wayland 出范围。
- 公开入口文档改用英文（`README.md` / `QUICKSTART.md` / `PLATFORMS.md`），内部设计文档保持中文。

## 尚未覆盖（0.3.0 收尾时补齐）

- 文本 API 的 `measure_with(avail_width)`（S2 并行文本管线的前置）；
- 其余公开结构体的 `#[non_exhaustive]` 批处理（`Options`、`Theme`、事件结构等）；
- 复杂文本（shaping）、无障碍 OS 桥（AccessKit）、Linux AT-SPI —— 见路线图 S2/S3。

### 5. 新增 `y901-a11y` crate（第 12 个 workspace 成员）

无障碍桥是**独立 crate**，不进默认依赖树：

```toml
[dependencies]
y901 = { version = "0.3.0", features = ["a11y"] }   # 或直接依赖 y901-a11y
```

```rust
let update = y901::a11y::to_tree_update(tree.snapshot());
```

平台适配器再加一层 feature：Windows `y901-a11y/windows-uia`、
Linux `y901-a11y/unix-atspi`。**默认关**是刻意的：不需要 a11y 的应用不拉
`accesskit` / `zbus`。

### 6. 发布顺序多了一步

`y901-a11y` 依赖 `y901-ui`，必须排在 `y901-widgets` 之前发布
（`scripts/publish-crates.ps1` 与 `check-packaging.ps1` 的依赖序已同步，共 12 个 crate）。