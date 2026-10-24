# Y901 框架总览

Y901 是一个**零依赖、Agent 原生**的桌面 UI 框架。UI 框架本身不认识任何平台 API，
平台能力（Win32 / D3D11 / DirectWrite / PTY）全部收敛在应用层。

## 1. 六个 crate

```
y901-core     纯逻辑：终端 Grid / reflow / scrollback（依赖 unicode-width）
y901-ui       框架：Widget / UiTree / 布局 / 事件 / Signal / 语义树（零依赖）
y901-widgets  官方组件：Terminal / TextInput / Button / Label / List / ScrollView / SplitView / Menu / Dialog
y901-agent    安全网关：语义 API、Token、作用域、版本锁、脱敏
y901-mcp      MCP 桥：stdio + JSON-RPC 2.0，把网关暴露给 Claude Desktop / Cursor
y901-app      应用壳：Win32 窗口 + D3D11 后端 + 事件循环
```

依赖方向严格单向：

```
y901-app -> y901-mcp -> y901-agent -> y901-ui <- y901-widgets -> y901-core
```

`y901-ui` 的 `[dependencies]` 是空的。这不是洁癖，而是测试策略的地基：
156 项测试里绝大多数不需要窗口、GPU、PTY。

## 2. 两个桥

框架不直接调用平台 API，全部通过 trait 注入：

```rust
pub trait TextMetrics { fn measure_text(...); fn line_height(...); fn scale() -> f32; }

pub trait RenderCtx: TextMetrics {
    fn fill_rect(...); fn stroke_rect(...); fn draw_text(...);
    fn push_clip(...); fn pop_clip(...);
    fn theme(&self) -> &Theme;
}
```

`y901-app::backend::D3dBackend` 是唯一实现者；测试里用 `Recorder` 实现，
所以布局与绘制可以在无窗口环境断言。

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

`s_y901-agent` 提供语义 API（不传坐标）：

| 工具 | 参数 |
|---|---|
| `ui_click` | `id`, `if_rev?` |
| `ui_type_text` | `id`, `text`, `if_rev?` |
| `ui_scroll` | `id`, `delta`, `if_rev?` |
| `ui_invoke` | `id`, `action`, `args`, `if_rev?` |

安全检查（顺序即优先级）：

1. **Token**：`--mcp` 时读 `Y901_TOKEN`，请求需带 `params.token`（-32001）；
2. **能力作用域**：`exposed_to_ai` 默认 **false**，组件必须显式 `.exposed(true)`；
3. **版本锁**：`if_rev` 与节点 `revision()` 不符 → 409；
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
cargo test --workspace     # 156 项
cargo build --workspace    # 零警告
```

绝大多数测试无需窗口/GPU：布局、事件路由、裁剪命中、语义树、网关鉴权、
MCP 协议往返、图集打包都能在 CI 里跑。