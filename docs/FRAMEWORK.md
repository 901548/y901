# Y901 框架总览

Y901 是一个**零依赖、Agent 原生**的桌面 UI 框架。UI 框架本身不认识任何平台 API，
平台能力（Win32 / D3D11 / DirectWrite / PTY）全部收敛在应用层。

## 1. 八个 crate

| crate | 职责 | 生产依赖 |
|---|---|---|
| `y901-core` | 纯逻辑：终端 Grid / reflow / scrollback / 格宽（`width_of_char`） | `unicode-width` |
| `y901-ui` | 框架：Widget / UiTree / 布局 / 事件 / Signal / 语义树 / revision | **无** |
| `y901-widgets` | 官方组件：Terminal / TextInput / Button / Label / List / ScrollView / SplitView / Menu / Dialog | `core`, `ui` |
| `y901-agent` | 安全网关：语义 API（`dispatch_request`）、Token、作用域、版本锁、脱敏；RPC 与 MCP 协议 | `ui` |
| `y901-runtime` | 平台无关契约：`Host` / `PlatformRuntime` / `RawEvent` / `translate` / `RenderBackend` | `ui`, `agent` |
| `y901-mcp` | MCP 桥：stdio + JSON-RPC 2.0，把网关暴露给 Claude Desktop / Cursor | `agent`, `ui`, `widgets` |
| `y901-backend-win32` | 唯一接触 Win32 的 crate：窗口 / D3D11 / DirectWrite / IME / DPI | `core`, `ui`, `runtime`, `windows-sys` |
| `y901-app` | 启动器：解析参数、建树、装配 Host、跑事件循环（不实现组件、不定义布局） | `backend-win32`, `runtime`, `mcp`, `agent`, `ui`, `widgets` |

依赖方向严格单向（箭头指向被依赖方）：

```
y901-app ──> y901-backend-win32 ──> y901-runtime ──> y901-agent ──> y901-ui
    │                │                    │                │
    │                └──> y901-core <─────┴──> y901-widgets ┘
    ├──> y901-mcp ──> y901-agent / y901-widgets / y901-ui
    └──> y901-widgets / y901-ui
```

关键约束：

- `y901-ui` 的 `[dependencies]` 是**空的**。这不是洁癖，而是测试策略的地基：
  278 项测试里绝大多数不需要窗口、GPU、PTY。
- 只有 `y901-backend-win32` 依赖 `windows-sys`（grep 可验证）。
- `y901-runtime` 单独存在的原因：`Host::policy()` 需要 `y901_agent::Policy`，
  而 `agent` 依赖 `ui`；若把 `Host` 放进 `ui` 会形成 `ui -> agent -> ui` 循环。

## 2. 两个桥（UI 层）与一层平台契约

**UI 层两个 trait**（`y901-ui`）：

```rust
pub trait TextMetrics { fn measure_text(...); fn line_height(...); fn scale() -> f32; }

pub trait RenderCtx: TextMetrics {
    fn fill_rect(...); fn stroke_rect(...);
    fn draw_text(...);                     // 画一串文本
    fn draw_char(...);                     // 画单字符（默认实现用栈缓冲，零分配）
    fn push_clip(...); fn pop_clip(...);
    fn theme(&self) -> &Theme;
}
```

`y901-backend-win32::D3dBackend` 是生产实现；测试里用 `Recorder` 实现，
所以布局与绘制可以在无窗口环境断言。

**平台契约**（`y901-runtime`）：

```rust
pub trait Host {
    fn idle(&mut self, tree: &mut UiTree);
    fn command(&mut self, tree: &mut UiTree, cmd: Command) -> bool;
    fn event(&mut self, tree: &mut UiTree, ev: &UiEvent) -> EventResult;   // 默认 Ignored
    fn policy(&self) -> &Policy;
    fn ime_position(&self) -> Option<Pos>;
}

pub trait PlatformRuntime {
    fn run(host: Box<dyn Host>, build: Box<dyn FnOnce() -> UiTree>, options: Options);
}

pub trait RenderBackend {
    fn create(handle: PlatformWindowHandle, w: u32, h: u32) -> Result<Self, String>;
    fn rebuild(&mut self, w: u32, h: u32) -> Result<(), String>;
    fn resize(&mut self, w: u32, h: u32) -> Result<(), String>;
    fn present(&mut self, frame: &Frame) -> Result<(), String>;
    fn is_occluded(&self) -> bool;
    fn instance_count(&self) -> usize;
}
```

`RenderBackend` 是跨平台 v3 的第 1 步：绘制指令（`GfxInstance` / `AtlasUpdate`）与平台无关，
`RecordingBackend` 提供"只记录不渲染"的 mock，用于对拍。


## 3. 组件模型

```rust
pub trait Widget {
    fn id(&self) -> &str;
    fn role(&self) -> Role;
    fn layer(&self) -> Layer;          // Base / Overlay / Modal
    fn is_open(&self) -> bool;         // 关闭的浮层不进索引
    fn focusable(&self) -> bool;
    fn is_disabled(&self) -> bool;
    fn clip(&self) -> Option<Rect>;    // 滚动容器声明视口
    fn cursor_at(&self, pos: Pos) -> Cursor;

    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size;
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache);
    fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult;
    fn render(&mut self, ctx: &mut dyn RenderCtx);
    fn semantics(&self) -> Semantics;
    fn on_mount(&mut self, ctx: &mut MountCtx);
    fn on_unmount(&mut self);
}
```

## 4. 坐标系与裁剪

- `rect()` 是**父坐标系**（arrange 传入的绝对矩形）；
- 滚动容器把子节点排在**内容坐标系**（`y - offset`），用 `clip()` 声明视口；
- `hit_index` 用 `rect ∩ clip` 判定可见性 —— 看不见就点不到；
- 渲染时框架自动 `push_clip` / `pop_clip`，组件无需手写。

## 5. 事件：三阶段

```
命中 -> Capture(根→父) -> Target -> Bubble(父→根)
```

- **Capture 用于拦截，Bubble/Target 用于响应**；在 Capture 里做响应会导致执行两次。
- `ctx.set_pointer_capture(id)`：拖拽期间 Events 强制投递（SplitView、滚动条依赖它）；
- `ctx.veto_focus()`：焦点守卫；
- Modal 层吞噬外部点击，Focus 被限制在子树内。

## 6. 响应式：Signal

```rust
let title = Signal::new("hello".to_string());
let label = TextLabel::new("t", "hello").bind(title.clone());
// 之后只需：
title.set("world".to_string());
```

- `Signal` 不认识 `UiTree`：值变化只把订阅者 id 推进 `Invalidation` 队列；
- 组件在 `on_mount` 订阅、`on_unmount` 退订（`bind()` 后自动）；
- 框架在 `layout()` / `render()` 开头排空队列并标脏；
- 同帧内多次 set 去重为一次失效。

## 7. 动态挂载

```rust
tree.add_node("panel", Box::new(Button::new("ok", "OK")));
tree.insert_node("panel", 1, Box::new(TextLabel::new("x", "hi")));
tree.remove_node("ok");
```

- `add_node` / `insert_node` 会触发 `on_mount`、标记父节点布局失效；
- `remove_node` 递归 `on_unmount`，并清理落在被删子树里的焦点与指针捕获。

## 8. 组件用法速览

```rust
// 滚动容器
let list = ScrollView::new("sv", Box::new(content)).flex_item(FlexItem::fill());

// 虚拟化列表（10k 行只排布可见的几行）
let list = ListView::new("files", 20.0)
    .items(names)
    .selected(Some(0))
    .on_row_selected(|i| println!("{i}"));

// 分栏
let panes = SplitView::horizontal("split", Box::new(left), Box::new(right)).ratio(0.3);

// 下拉菜单（Overlay 层）
let mut menu = Menu::new("menu").items(vec!["Open".into(), "Quit".into()]);
menu.open(); // 键盘 Down/Up 移动高亮，Enter 触发

// 对话框（Modal 层）
let mut dlg = Dialog::new("dlg", "Confirm")
    .push(Box::new(TextLabel::new("body", "Sure?")))
    .buttons(vec![("Cancel", "cancel"), ("OK", "ok")]);
dlg.open();
```

**宿主约定**：Menu / Dialog 需要全屏宿主才有"点外部关闭/吞噬"的几何依据，
用 `Stack`（所有子节点同尺寸）把它们放在最外层。

## 9. Agent 网关与 MCP

`y901-agent` 提供语义 API（不传坐标）。RPC 与 MCP 共用同一个入口
`dispatch_request(tree, policy, method, params)`，方法名同时接受短名与工具名：

| 工具 | 参数 | 说明 |
|---|---|---|
| `ui_click` | `id`, `if_rev?` | 点节点中心 |
| `ui_type_text` | `id`, `text`, `if_rev?` | 输入文本 |
| `ui_scroll` | `id`, `delta`, `if_rev?` | 滚动 |
| `ui_query` | `q` | 按选择器筛选（`role=button & label~="Save" & limit=5`） |
| `ui_invoke` | `id`, `action`, `args`, `if_rev?` | 调用声明的动作 |
| `ui_tree` | — | 整棵语义树（已过滤 + 脱敏 + 带 revision/focused） |
| `ui_node` | `id` | 单节点（MCP resource：`ui://node/{id}`） |

安全检查（顺序即优先级）：

1. **Token**：`--rpc` 每请求校验 `params.token`；`--mcp` 校验**环境变量**
   `Y901_TOKEN`（MCP 的 `initialize` 没有 token 字段，不能用协议层传递）；
2. **能力作用域**：`exposed_to_ai` 默认 **false**，组件必须显式 `.exposed(true)`；
   连 `dispatch` / `focus` / `subscribe` 也过这道闸（`unsubscribe` 例外）；
3. **版本锁**：`if_rev` 与节点**有效** revision（组件自报 + 框架计数器）不符 → 409；
4. **脱敏**：`sensitive` 节点的 `label` 与 `value` 一律 `***`，`state` 逐键打码。


### Claude Desktop 配置

```json
{
  "mcpServers": {
    "y901": {
      "command": "F:\\Y901\\target\\release\\y901-app.exe",
      "args": ["--mcp"],
      "env": { "Y901_TOKEN": "local-dev-token" }
    }
  }
}
```

- `--mcp`：把正在运行的窗口挂到 stdio；
- `--mcp --headless`：不开窗口，纯协议（CI 用）；
- `--rpc`：启用 TCP 通道（9410），**必须**配置 `Y901_TOKEN`，且每请求校验。

## 10. 测试

```
cargo test --workspace          # 278 项（含 1 个需真机的 #[ignore]）
cargo build --workspace         # 零警告
cargo clippy --workspace --all-targets   # 零警告
scripts/test-count.ps1          # 唯一可信的计数方式（把输出写文件再解析）
```

绝大多数测试无需窗口/GPU：布局、事件路由、裁剪命中、语义树、网关鉴权、
MCP 协议往返、图集打包、revision 语义面哈希、RPC 写队列边界都能在 CI 里跑。

需要真机的测试只有 `rebuild_reuploads_atlas`（D3D11 设备重建后图集重传），
在无显卡权限的机器上会打印 `[skip]` 并返回。