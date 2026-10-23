# Y901 框架总览

> 一句话：**Y901 是一个零依赖的 Agent-Native 桌面 UI 框架**。
> 平台能力（Win32 / D3D11 / DirectWrite / PTY）全部在应用层，框架只认抽象。

```
┌──────────────────────────────────────────────────────────┐
│  Agent (LLM / 脚本)                                       │
│      │  TCP 127.0.0.1:9410  · JSON-RPC                    │
│      ▼                                                    │
│  y901-ui        框架：Widget / UiTree / Flex / 事件 / 语义 │  ← 零依赖
│      ▲                                                    │
│  y901-widgets   组件：Terminal / TextInput / Button …      │  ← 只依赖 y901-ui
│      ▲                                                    │
│  y901-app       应用：Win32 窗口 + D3D11 后端 + RPC 总线    │  ← 唯一碰平台的地方
└──────────────────────────────────────────────────────────┘
```

## 1. 四个 crate 的边界

| crate | 职责 | 允许依赖 | 绝对不做 |
|---|---|---|---|
| `y901-core` | 纯逻辑终端内核：`Grid` / reflow / scrollback / 样式表 | `unicode-width` | 不画图、不开窗 |
| `y901-ui` | 框架：`Widget` / `UiTree` / 布局 / 事件 / 语义树 / RPC 协议 | **无** | 不认识 Win32、D3D11、PTY、任何业务 |
| `y901-widgets` | 官方组件库 | `y901-core`、`y901-ui`、`portable-pty` | 组件之间互不认识 |
| `y901-app` | 应用壳：窗口、渲染后端、RPC 总线、IME | 上面三个 + `windows-sys`、`crossbeam-channel` | 不含组件逻辑 |

依赖树（`cargo tree --depth 1`）：

```
y901-app
├── crossbeam-channel
├── windows-sys
├── y901-ui          (无依赖)
└── y901-widgets

y901-core
└── unicode-width
```

## 2. 框架提供给组件的两座桥

框架不直接调用任何平台 API，全部通过 trait 注入：

```rust
/// 布局期：把文本量成像素
pub trait TextMetrics {
    fn measure_text(&self, text: &str, style: &TextStyle) -> Size;
    fn line_height(&self, style: &TextStyle) -> f32;
    fn scale(&self) -> f32;
}

/// 渲染期：把绘制指令交给后端
pub trait RenderCtx: TextMetrics {
    fn size(&self) -> Size;
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32;
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);
}
```

`y901-app` 里的 `backend::D3dBackend` 是**唯一**实现者：`fill_rect` / `draw_text` 累积成实例数组，
每帧一次 `DrawInstanced` 提交换链；字形由 DirectWrite 光栅化进 RTL 图集（skyline 打包 + 压缩搬移 + LRU）。

换成软件后端、测试记录器、甚至 Web Canvas，组件一行都不用改 —— `crates/y901-ui/tests/framework.rs`
里的 `Recorder` 就是这么跑的（无窗口、无 GPU）。

## 3. 组件模型

```rust
pub trait Widget {
    fn id(&self) -> &str;
    fn role(&self) -> Role;                        // 语义角色，开放字符串
    fn flex(&self) -> FlexItem;                    // 伸缩参数
    fn layer(&self) -> Layer;                      // Base / Overlay / Modal
    fn focusable(&self) -> bool;                   // Tab 遍历与 Modal 自动聚焦
    fn layout_version(&self) -> u64;               // 内在尺寸版本（布局缓存键）
    fn revision(&self) -> u64;                     // 语义版本（增量推送用）

    fn children(&self) -> &[Box<dyn Widget>];
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>>;

    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size;
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache);
    fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult;
    fn render(&mut self, ctx: &mut dyn RenderCtx);
    fn semantics(&self) -> Semantics;
}
```

生命周期：`measure → arrange → render`；事件随时来。容器在 `arrange` 里继续给子节点排布
（框架提供 `layout::flex_arrange`），递归由容器驱动。

## 4. 布局：手写 Flex + 记忆化

- 只有 **Row / Column** 两种方向，没有 wrap / absolute / aspect-ratio。
- 容器必须通过 `LayoutCache::measure(child, …)` 访问子节点，框架在那里按
  `(id, constraints, layout_version)` 记忆。**相同约束下第二次布局是 O(1)**，
  2000 节点树的测试断言 `measure` 调用次数为 0。
- `Constraints::unbounded_main(axis, …)` 显式表达"主轴无界"（ScrollView 场景），
  返回值永远是有限内容尺寸，不会把 `inf` 泄漏给 GPU。

## 5. 事件：三阶段 + 指针捕获 + 分层

```
命中(hit_index，零分配) → Capture(根→父) → Target → Bubble(父→根)
```

- `EventCtx::stop_propagation()` 立刻停止后续阶段；
- `EventCtx::set_pointer_capture(id)` 让之后的 `MouseMove` / `MouseUp` 无视坐标强制投递
  （TextInput 拖拽选中就靠它）；
- 层级 `Base → Overlay → Modal`：绘制正序、命中逆序；Modal 外部点击被吞掉，焦点被限制在子树内；
- `Focus` 事件也走三阶段，组件可 `ctx.veto_focus()` 拒绝焦点离开（表单守卫）。

路径表示：`Vec<usize>`（子索引）。派发沿路径**递归一趟**，访问节点数 = 深度，
与树规模无关（测试：1000 个兄弟节点不改变访问次数）。

## 6. 语义树与 Agent 协议

`UiTree::snapshot_json()` 输出：

```json
{"id":"input","role":"text_input","rect":{"x":0,"y":0,"w":300,"h":32},
 "focused":true,"label":"Agent","value":"Agent",
 "actions":["set_text","focus","select_all"],
 "state":{"text":"Agent","caret":5,"anchor":5,"focused":true},
 "children":[]}
```

RPC（TCP，行分隔 JSON）：

| 方法 | 作用 |
|---|---|
| `get_tree` | 整棵语义树（含 rect / role / state） |
| `invoke {id, action, args}` | 触发组件动作 |
| `dispatch {id, event}` | 投递原生事件（text / key / mouse_down / mouse_up / mouse_move / wheel / compose） |
| `subscribe {id, interval_ms}` | 订阅状态变化，服务端主动推 `state_changed` |
| `unsubscribe {id}` | 取消订阅 |
| `focus {id}` | 切换焦点 |
| `ping` | 健康检查 |

推送是**差分 + 节流**的：`patch` 只含变化的字段，窗口内的多次变化合并成一条。
指纹由 `Widget::revision()` + `value` + `state` 三部分哈希而成。

## 7. 线程模型

- **UI 线程**独占 `UiTree` / `Grid` / `Gfx`，无锁；
- **PTY 读线程**：`read` → `crossbeam_channel` → `PostMessage(WM_APP_PTY_DATA)`；
- **RPC 线程**：只做解析与转发，业务在 UI 线程执行；
- 主循环 `PeekMessage` 排空后跑 `idle()`：抽取 RPC、`Tick` 驱动组件 pump、推送订阅、按需重绘。

> **关于唤醒**：框架**没有** `Waker` 抽象。暂停/唤醒由应用层实现 ——
> 后台线程用 `PostMessage` 打断 `PeekMessage` 循环，组件通过 `UiEvent::Tick` 主动轮询。
> 等真正需要"后台任务完成即重绘"时再引入 runtime 层（见白皮书 M7.8）。

### 7.1 推送策略：非阻塞广播 + 满队列丢弃

RPC 推送**绝不允许**在 UI 线程碰 socket。策略分三层：

| 层 | 做什么 | 失败时 |
|---|---|---|
| **UI 线程** | `Broadcaster::send(line)` → `SyncSender::try_send` | 队列满 → **直接丢弃**并计数，立即返回 |
| **有界队列** | `std::sync::mpsc::sync_channel(1024)` | 容量固定，不增长 |
| **写线程**（`y901-rpc-push`） | 从队列取 → 遍历连接池 `writeln!` + `flush` | 写失败 → 从连接池剔除该连接 |

设计取舍：

- **为什么丢弃而不是阻塞**：UI 线程是唯一能推进布局/渲染/事件的地方。为一个读得慢的
  Agent 卡住整个界面，是拿最贵的资源等最便宜的环节。丢一条 `state_changed` 的代价是
  Agent 少看到一帧中间态，下次推送会带上最新差分——**最终一致，不丢终态**。
- **为什么不用 crossbeam**：`std::sync::mpsc::sync_channel` 的 `try_send` 语义与
  crossbeam 的 `try_send` 完全一致（有界、非阻塞、返回 `Full`）。`y901-ui` 的零依赖是
  架构承诺，为一个等价 API 破坏它不划算。
- **诊断**：`Broadcaster::dropped()` 返回累计丢弃条数，可用于确认"是不是客户端太慢"。
  这是唯一的可观测面，不额外暴露队列长度（避免调用方依赖内部状态）。

实测基线（`broadcast_never_blocks_ui`）：容量 2、无消费线程时连发 1000 条，
总耗时 < 50ms，`dropped() == 998`。

> 未来若要"不丢"，正确做法是**每连接一个队列 + 写端合并**（同一 id 只保留最新 patch），
> 而不是加大缓冲或改成阻塞写。

## 8. 目录

```
crates/
  y901-core/     Grid · reflow · scrollback · StyleTable
  y901-ui/       widget · layout · tree · event · command · json · rpc · text
  y901-widgets/  terminal · text_input · button · label · list · vt · palette
  y901-app/      main（窗口/事件/RPC） · backend（d3d11 · font · atlas） · ime
docs/
  FRAMEWORK.md        本文件：架构总览
  FRAMEWORK_GUIDE.md  上手手册：写第一个组件、发命令、跑 RPC
```

## 9. 测试

```
cargo test --workspace      # 89 项，绝大多数不需要窗口/GPU
cargo clippy --workspace    # 仅剩风格类提示
```

布局、事件路由、语义树、RPC 往返、图集打包/压缩都能在 CI 里跑；
真实渲染与窗口行为靠 `cargo run -p y901-app` 验证。

### 7.2 通过 MCP 接入 AI 生态

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

- `--mcp`：把**正在运行的那棵 UiTree** 挂到 stdio（stdout 只走协议，日志全部 stderr）。
- `--mcp --headless`：不开窗口，纯协议模式（CI / 测试用）。
- 暴露的四种工具：`ui_click` / `ui_type_text` / `ui_scroll` / `ui_invoke`；
  两个资源：`ui://tree`、`ui://node/{id}`。