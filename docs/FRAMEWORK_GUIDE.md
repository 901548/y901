# Y901 组件开发指南

本指南面向"要写一个新组件/一个新应用"的开发者。

## 0. 五条铁律

1. **框架不认识平台**：画图走 `RenderCtx`，量字走 `TextMetrics`。
2. **框架不认识业务**：`y901-ui` 里没有任何终端/文件树/IDE 的概念。
3. **组件互不认识**：只通过 `UiEvent::Action` 与 `Command` 通信。
4. **Capture 拦截，Bubble 响应**：在 Capture 里做响应会执行两次。
5. **默认封禁**：`Semantics::default()` 的 `exposed_to_ai = false`，
   要给 AI 用必须显式 `.exposed(true)`。

## 1. 最小组件

```rust
struct Badge { id: String, text: String, rect: Rect }

impl Widget for Badge {
    fn id(&self) -> &str { &self.id }

    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        let s = m.measure_text(&self.text, &TextStyle::default());
        c.constrain(Size::new(s.w + 8.0, s.h + 4.0))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect { self.rect }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let st = ctx.theme().style(Role::TEXT, State::Normal);
        ctx.fill_rect(self.rect, st.bg);
        ctx.draw_text(Pos::new(self.rect.x + 4.0, self.rect.y + 2.0),
                      &self.text, &TextStyle::fg(st.fg));
    }

    fn semantics(&self) -> Semantics {
        Semantics::default().label(&self.text).exposed(true)
    }
}
```

要点：

- `measure` 只能用**传入的** `cache.measure(child, …)` 访问子节点，直接调子节点会绕过记忆化；
- 颜色一律从 `ctx.theme()` 查，不要硬编码；
- 交互组件记得 `.exposed(true)`，否则 AI 看不到也点不到。

## 2. 容器

```rust
fn children(&self) -> &[Box<dyn Widget>] { &self.kids }
fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> { Some(&mut self.kids) }
```

**返回 `None` 意味着框架永远下钻不进去**：子节点不会被渲染、不会被命中。
（ScrollView 曾经踩过这个坑。）

容器在 `arrange` 里自行给子节点排布，可以用 `layout::flex_arrange`。

## 3. 滚动容器

实现三件套：

```rust
fn clip(&self) -> Option<Rect> { Some(self.inner) }   // 视口

fn arrange(&mut self, rect: Rect, ...) {
    let content = cache.measure(child, m, Constraints::unbounded_main(Axis::Vertical, rect.size()));
    child.arrange(Rect::new(rect.x, rect.y - self.offset, rect.w, content.h), ...);
}

fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult {
    if ctx.phase == EventPhase::Capture { return EventResult::Ignored; }
    // 处理 Wheel → 改 offset → 返回 Relayout
}
```

滚动后必须返回 `EventResult::Relayout`：子节点位置变了，只 `Redraw` 会画错。

## 4. 响应式

```rust
pub struct Clock { id: String, now: Signal<String> }

impl Widget for Clock {
    fn on_mount(&mut self, ctx: &mut MountCtx) {
        self.now.subscribe(ctx.id.clone(), ctx.invalidation.clone());
    }
    fn on_unmount(&mut self) { self.now.unsubscribe(&self.id); }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let text = self.now.get();   // 每帧读最新值
        ctx.draw_text(Pos::new(self.rect.x, self.rect.y), &text, &TextStyle::default());
    }
}
```

`Signal::set` 之后**不需要**任何 `invalidate`：框架每帧自动排空失效队列。

## 5. 浮层

```rust
fn layer(&self) -> Layer { Layer::Overlay }   // 或 Modal
fn is_open(&self) -> bool { self.open }       // false 时不进索引、不拦截
```

- Overlay：绘制在最上层，点击可穿透；
- Modal：吞噬外部点击，焦点被限制在子树内；
- 两者都需要**全屏宿主**（`Stack`），否则 rect 会被父容器约束住。

## 6. 与 Agent 打交道

```rust
fn semantics(&self) -> Semantics {
    Semantics::default()
        .label("Save")
        .action("click")            // invoke 白名单
        .state("dirty", true)       // 结构化状态
        .exposed(true)              // 对 AI 可见
        .sensitive(false)           // true 则 label/value 打码
}
```

**版本锁**：`revision()` 变化代表语义状态变了。AI 带 `if_rev` 调用时会比对它，
不一致返回 409。文本类组件应在内容/光标变化时 bump。

### 6.1 顶层语义字段 vs `state` 字段

边界只有一条：**Agent 需要独立判断的东西放顶层，组件自有的结构化数据放 `state`**。

| 字段 | 位置 | 理由 |
|---|---|---|
| `label` / `value` | 顶层 | Agent 直接读 |
| `disabled` | **顶层** | "现在能不能点"，是独立语义 |
| `sensitive` / `exposed_to_ai` | **顶层** | 决定"看不看得到、要不要打码" |
| `actions` | **顶层** | 动作集 |
| 其余（`text` / `caret` / `anchor` / `open` / `selected` …） | `state` | 组件自定义 |

**禁止在 `state` 里重复顶层字段**（例如 `state("disabled", …)`）。原因：

1. Agent 会看到两份，无法判断哪个是权威；
2. revision 的哈希范围要覆盖二者时会重复计算，不覆盖时又会漏报；
3. 两份值漂移时，没有任何机制能发现。

同一条规则也适用于 `props`：`props` 只给调试看，**不进 Agent 输出，也不进哈希**。

## 7. 应用骨架

```rust
fn main() {
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(1200.0, 800.0), &metrics);
    loop {
        // 1) 取事件  2) tree.dispatch(&ev)
        // 3) if result.needs_relayout() { tree.layout(..) }
        // 4) if result.needs_redraw()   { tree.render(&mut backend) }
        // 5) tree.drain_commands() 处理业务命令
    }
}
```

`--mcp` 时把 `y901_mcp::bridge::spawn_io(stdin, stdout)` 拿到的 `Receiver<Job>`
存进 App，在空闲时 `bridge::pump(&mut server, &mut tree, &jobs, 8)`。

## 8. 常见坑

| 现象 | 原因 |
|---|---|
| 子节点不显示、点不到 | `children_mut()` 返回了 `None` |
| 滚动后点击错位 | 滚动没返回 `Relayout`，或没实现 `clip()` |
| 滚轮滚两倍 | 在 Capture 阶段也处理了事件 |
| AI 看不到组件 | 忘了 `.exposed(true)`（默认封禁） |
| 焦点丢失 | 删节点时框架会清焦点，需要重新 `set_focus` |
| Signal 不生效 | 组件没在 `on_mount` 里 `subscribe` |

## 9. 测试建议

- 布局/命中：构造 `UiTree`，`layout` 后断言 `rect_of` / `hit_test`；
- 事件：用 `tree.dispatch_to(id, &event)` 走真实三阶段；
- 渲染：实现一个记录型 `RenderCtx`，断言 `draw_text` 收到的内容；
- 网关：直接调 `y901_agent::handle_call`，不需要起 TCP。

## 10. id 命名规范

id 是 Agent 定位节点的**唯一稳定锚点**，也是 `query` 的筛选依据。建议按路径式命名：

| 类型 | 规范 | 示例 |
|---|---|---|
| 面板/容器 | `panel.{name}` | `panel.sidebar`、`panel.editor` |
| 列表项 | `list.{list}.item.{n}` | `list.files.item.12` |
| 操作按钮 | `action.{verb}` | `action.save`、`action.cancel` |
| 输入框 | `input.{field}` | `input.username`、`input.search` |
| 弹层 | `menu.{name}` / `dialog.{name}` | `menu.file`、`dialog.confirm` |
| 状态区 | `status.{area}` | `status.bar` |

约定：

1. **扁平、可读、不以数字开头**；
2. 列表项用**数据索引**而不是屏幕序号（滚动后不变）；
3. 同一个 id 在树里**唯一**——重复会让 `find`/`dispatch_to` 命中第一个；
4. 动态内容（行、页签）把变化的部分放到最后一段。

配套的查询示例：

```
role=button & id~="action."
label~="Save" & limit=5
panel.editor 里的一切：id~="panel.editor"
```

框架不强制校验命名，但 `query` 的表达式能力建立在命名可预测之上——
命名混乱时 AI 只能拉全树，方向 C 的收益就没了。

## §12 待清理的语义一致性问题

> **状态：已解决（2026-10）。** 保留原文作为历史记录。
>
> - `focused` 已统一为**顶层语义字段**，由 `UiTree::focus_path` 注入（见 §14）；
>   `TextInput` / `Terminal` 的内部 `focused` 只用于渲染与按键门控，不再进 `state`。
> - `disabled` 同样只出现在顶层（`sem.disabled`），`state` 里不再有它。
> - `props` 仍未进哈希也未进 Agent 输出，但它是调试信息，不属于"语义重复"。

以下为**历史原文**（当时未修复）：

- **`focused` 字段的不一致**：`TextInput` 把 `focused` 放在 `state` 里，
  而 `Button` 没有这个概念。需要先确认「focused 是否应作为顶层语义字段」：
  若是，统一提到顶层（与 `disabled` 同规则）；若否，则明确各组件职责边界。
  在此之前不要新增依赖 `state.focused` 的 Agent 脚本。

- **`props` 的 `focused` / `hovered` 双写风险**：`TextInput` 同时在 `props`
  和 `state` 里输出 `focused`。`props` 不进哈希也不进 Agent 输出，
  因此不算 bug，但属于可清理的冗余。

发现新的同类问题时，请追加到本节，而不是就地打补丁——
乱改会让 revision 的哈希范围再次含糊。
## §13 `render()` 必须是纯函数

**规则**：`render()` 只能读状态、只能通过 `ctx` 绘制。不得：

1. 修改组件自己的任何字段（包括"顺手"缓存一个测量结果）；
2. 调用 `tree.dispatch(..)` 或触发任何事件；
3. 读取外部可变状态（`RefCell` / `static mut` / 全局计数器）；
4. 分配资源（创建纹理、打开文件、spawn 线程）。

**为什么**：revision 机制的哈希时机在 `layout()` 开头（一帧一次）。
如果组件在 `render()` 里改状态，这次变化**不会被哈希捕获**——
Agent 看到的 `revision` 不变，但 `get_tree` 的内容变了，
乐观锁（`if_rev`）就会在错误的时刻放行。

**允许改状态的地方**：`event()`、`on_mount()`、`on_unmount()`。
这三处都会被框架的版本机制覆盖。

**自检方法**：把 `render()` 的 `&mut self` 临时改成 `&self` 编译一次
（或在其中只调用 `&self` 方法），若编译不过，说明它偷偷改了状态。
## §14 `focused` 的语义

`focused` 是**框架拥有的语义字段**，由 `UiTree::focus_path` 注入，组件不得设置。

| 位置 | 含义 | 维护者 |
|---|---|---|
| 顶层 `Semantics.focused` | 该节点当前是否接收键盘事件 | 框架（`set_focus` / Tab / Modal / RPC focus） |
| 组件内部 `focused` 字段 | 渲染状态（光标闪烁、按键门控） | 组件自己，靠 `UiEvent::Focus` 同步 |

**规则**
1. Agent 判断"输入会到哪"应看**顶层** `focused`，不要看 `state`。
2. 组件**不得**在 `request_focus` 之前就把自己的 `focused` 置 true：
   请求可能被 Modal 作用域拒绝，抢跑会导致"组件自以为聚焦、框架却把按键发给别人"。
   正确做法：只 `request_focus`，等框架的 `Focus(true)` 事件回来再置位。
3. 顶层 `focused` 参与 revision 哈希：焦点变化 = Agent 可操作对象变化。
4. 订阅 patch 覆盖 `value / state（拍平）/ disabled / focused / label`。
## §15 订阅 patch 的形状

`state_changed` 推送里的 `patch` 是**拍平**的：`state` 内的字段被提到顶层。

```json
{"event":"state_changed","id":"input","rev":3,
 "patch":{"text":"abcXY","caret":5,"focused":true}}
```

- 访问方式：`patch.text`，**不是** `patch.state.text`（与 `get_tree` 的嵌套结构不同）。
- 覆盖字段：`value` / `state` 内各键 / `disabled` / `focused` / `label`。
- 只包含**真正变化**的字段；未变化的字段不出现。
- 设计理由：订阅推送只服务于"增量更新"，拍平能让消费者直接按键取值，
  无需先判空 `patch.state`。

顺带一条顺序保证：**回包永远先于该回包触发的推送**。
实现上 `reply_to` 在 UI 线程内同步写 socket，推送随后才入广播队列；
早期实现里两者由不同线程写，出现过"先收到 state_changed 再收到 ack"的乱序。
## §16 已实现：RPC 回包已移出 UI 线程

> **状态：已实现（2026-10）。** 保留迁移记录作为设计依据。

**现状**：每连接一个写线程 + 统一 FIFO 队列（`WriteJob::{Reply, Push}`）。
`reply_to` 只做 `try_send`，**不碰 socket**；队列满即标记 `dead` 并返回 `Err`，
写线程看到 `dead` 后 `shutdown`，客户端立刻收到 EOF（B+ 快速失败）。
`set_write_timeout(100ms)` 仍在，但作用于**写线程**（让它周期性醒来检查 `dead`），
不再是"UI 线程的最坏阻塞"。

**为什么当初需要它**：UI 线程同步写 socket 时，客户端不读会把缓冲写满，
100ms 的写超时仍是一帧掉 6 帧。现在 UI 线程零阻塞，该风险消失。

**当时的迁移步骤（已完成）**
1. `Pending` 携带该连接的写队列 `Sender<WriteJob>`。✅
2. `Broadcaster` 从"持 socket"改为"持每条连接的 Sender + dead 标志"。✅
3. 每连接一个写线程：`for job in rx { write }`，写失败即断开并注销。✅
4. 删除 UI 线程的 socket 写；`set_write_timeout` 改为写线程属性。✅
5. 测试：`rpc_writer_thread.rs` / `rpc_dead_connection.rs` 断言队列语义。✅

### §16.1 TODO：`Pending.dead` 的可见性

`Pending.dead: Arc<AtomicBool>` 目前是 `pub`，外部可以读、也可以**误置 true** 踢掉健康连接。

当前风险低（调用方只有 `host.rs` 与 `rpc.rs` 内部，测试需要能构造 dead 连接），
但更稳妥的形态是：

- `dead` 私有 + `pub fn is_dead(&self) -> bool` 只读访问器；
- 测试改用 `#[doc(hidden)] pub fn mark_dead_for_test()` 构造。

改动成本约 10 行，等有第二个真实调用方时再做。
## §17 跨平台 TODO（不阻塞 RenderBackend 抽象）

- **`unicode-width == 0` 的字符两侧行为不一致**：`y901-core` 的 `place_cell`
  跳过它们（不占格），而 `draw_text` 里的 `span.max(1)` 会推进 1 格。
  当前影响只限 widget 多字符文本（终端每格单字符不受影响）。
  `max(1)` 是防死循环的必要防御，**不改**；正确修复是"span=0 时不推进，
  但用独立循环保护"，留待后续。

- **`glyph_offset` 只补偿水平方向**：垂直居中（baseline 与格高不符）未处理。
  Linux/macOS 的后备字体链可能在垂直方向同样需要补偿。等第二个后端落地时一并验证。
## §18 TODO：RenderBackend 的图集更新路径不一致

`D3dBackend` 通过 `TextEngine::sync_atlas()` 内部处理图集更新，**忽略** `Frame.atlas_updates`。
trait 契约要求后端从 `Frame` 读脏矩形，但 D3D11 的图集归 `TextEngine` 所有。

第 3 步对拍时，`RecordingBackend` 的 `atlas_updates` 会和 `D3dBackend` 不一致——
这正是抽象层没抽干净的信号，不是 mock 写错了。

**正确修复**：抽 `GlyphProvider` 时把图集管理从 `TextEngine` 提到 trait 接口里，
让"字形光栅化 + 图集分配 + 脏区上传"成为后端契约的一部分。
## §19 真机测试：`rebuild_reuploads_atlas`

`cargo test -p y901-backend-win32 rebuild_reuploads_atlas -- --ignored --nocapture`

需要**有 GPU 访问权限的桌面会话**。当前开发机是无显卡直通的虚拟显示适配器
（`GameViewer Virtual Display Adapter`），`D3D11CreateDeviceAndSwapChain` 连 WARP
兜底都返回 `0x80070005 (E_ACCESSDENIED)`，因此测试会打印 `[skip]` 并返回。

- 在有独显/核显的机器上：应当通过（断言 `rebuild` 后 `has_atlas()` 为真）。
- 在 CI/容器/远程会话：`[skip]` 是预期行为，不算失败。
- `present` 里的图集未同步检查在 release 下会打**一次**警告（`ATLAS_WARNED`）：
  看到这条日志说明 `rebuild` 逻辑有 bug，不是正常现象。
## §20 TODO：`set_focus` 无法区分失败原因

`UiTree::set_focus` 返回 `bool`，无法区分三种失败：
`Vetoed`（被 `ctx.veto_focus()` 拒绝）、`NotFocusable`（节点没有 `is_focusable`）、
`NotFound`（id 不存在）。

因此 `dispatch_request` 的 `focus` 方法在两种情况下都返回同一句
`"focus vetoed: {id}"`，文案略误导（可能是"不可聚焦"而非"被拒绝"）。

修法：`set_focus -> Result<(), FocusError>`，其中

```rust
pub enum FocusError { NotFound, NotFocusable, Vetoed }
```

`dispatch_request` 据此映射 404 / 400 / 409。
改动会波及所有调用点（Tab 遍历、Modal 聚焦、RPC focus），故不在 P0 轮次做。
## §21 TODO：结构测试依赖源码切片

`setup_connection_propagates_write_timeout_failure` /
`setup_connection_propagates_spawn_failure` 用 `include_str!` + `find("\n}")` 切出方法体。
这依赖"方法体里没有顶格 `}`"（例如嵌套宏、`macro_rules!` 展开）。

若未来方法体变复杂，需要改成 AST 级检查（`syn` crate），成本较高，留待后续。
在那之前：**修改 `setup_connection` 的方法体格式时，记得跑这两条测试**。
## §22 行为变化记录：预编辑串的宽度判定

**改动时间**：2026-10（P1-2，与 `draw_char` 同批）。

**旧行为**：IME 预编辑串按 `if ch as u32 > 0x2E80 { 2.0 } else { 1.0 }` 推进。
这是一个硬编码启发式：emoji（U+1F600）落在 0x2E80 以下 → 被当成 **1 格**，
而网格里它可能是 2 格，导致预编辑串与最终上屏位置错位。

**新行为**：`pen += cell_w * y901_core::Grid::width_of_char(ch)`，
与网格存储**同源**（unicode-width）。emoji 现在占 2 格；组合字符宽度 0 不推进。

**谁受影响**：终端里输入 emoji（或其它宽字符）时的预编辑显示位置。
中文/ASCII 不受影响（它们的宽度判定本来就一致）。

**为什么会顺手改**：它就与被替换的 `to_string()` 在同一行循环里。
按纪律报备：这是**超出"消除分配"范围的行为变化**，如需独立提交可回退。

## §23 TODO：`draw_char` 的"零分配"未实测

`RenderCtx::draw_char` 的默认实现用栈缓冲（`[u8; 4]` + `encode_utf8`），
理论上零堆分配。但**没有用分配计数器验证**：

- 替换 `#[global_allocator]` 会污染整个测试二进制；
- `String` 分配可能来自框架其它层，难以归因到"这一格"。

当前防线是结构测试 `terminal_render_has_no_to_string`（`render` 方法体里无
`to_string()`）。它的弱点是改注释也可能骗过。

**若要坐实**：用 `dhat` 或自定义 allocator + 独立测试二进制，测"渲染一帧
80×24 终端的分配次数"。成本约半天，收益是能抓住"未来有人在渲染路径里
偷偷分配"的回归。
## §24 为什么 `cursor_at` 要累加 `inherited_clip`（**防御性，不是修 bug**）

`cursor_at` 的递归里，每个祖先都会先检查
`node.rect().intersect(可见clip).contains(pos)`，不在可见区就**直接返回**——
子孙根本不会被访问。因此"是否把祖先 clip 累加到 `inherited_clip`"对最终
`contains(pos)` 的判定是**等价的**（早退 ≡ 累加）。

曾经怀疑这是 bug（"不累加祖先 clip 会让视口外的子节点贡献光标"），
实测与推导都证明**不成立**：`cursor_at_respects_ancestor_clip` 在修复前就是绿的。

那为什么仍然显式累加？

1. 与 `rebuild_indices` 的 `entry.clip` **同构**，便于两个函数对照；
2. 防止未来有人为了"性能"去掉祖先早退——届时若 `inherited_clip` 没累加，
   就会出现"祖先可见区不含 pos，但子孙仍被访问"的错误。

**改 `cursor_at` 的递归结构时，必须跑 `cursor_at_respects_ancestor_clip`，
并确认早退与累加仍然一致。**

## §25 TODO：`cursor_at` 的复杂度是 O(n)

每次 `MouseMove` 遍历全树。10k 节点树约 1ms/次，高频移动可能有感知延迟。
（旧实现也是 O(n)，本轮修复未变差。）

优化方向：复用 `hit_index` 的命中路径——先取命中节点，再只沿它的祖先链问 cursor
（`hit_index` 是 O(命中数)）。注意祖先链上的 cursor 继承语义要一并保留。

当前不优化。
## §26 `action_spec_named`：自定义动作参数名

默认情况下，动作的入参名由类型决定（`InputKind::Text` → `text`，
`Number` → `value`，`Bool` → `flag`，`None` → 无参数）。
当参数名不落俗套时（例如 `set_color` 的参数应该叫 `color`），用：

```rust
Semantics::default()
    .action_spec_named("set_color", InputKind::Text, "color")
```

- 网关按 `ActionSpec::arg_name()` 从 `args` 取键（自定义名优先，否则回退默认名）；
- `to_json` 只在**自定义**参数名时额外输出 `"arg"` 字段，
  默认名的动作 JSON 形状不变（既有 Agent 解析不受影响）；
- 参数类型校验（`accepts`）不变，仍然按 `InputKind`。

**不要**为了换个参数名而改 `InputKind`——那会同时改变类型校验。
## §27 真机验证清单（需要 GPU）

以下行为在 CI 与当前开发机上**无法验证**（虚拟显卡拒绝 D3D11 设备创建，
`D3D11CreateDeviceAndSwapChain` 返回 `0x80070005`），需要用户在真实 GPU 环境跑一次：

1. `Options { theme: Some(Theme::LIGHT), .. }` → 窗口真的变亮
   （不只是 `D3dBackend.theme` 字段变了）。
2. `Options { font: Some(FontConfig { family: "JetBrains Mono".into(), size: 20.0 }), .. }`
   → 字号真的变大，`cell_width ≈ 10.0`。
3. 终端里按 Enter → 命令只执行一次（控制键去重生效；此前是双写）。
4. 终端里按 Tab → shell 补全生效，且**不切换焦点**。
5. `Host::take_font_request()` 返回 `Some(20px)` → 字号真的变大，所有组件重排，无错位
   （`D3dBackend::cell` 与 `TextEngine` 的格子必须一致；此前修过一次错位 bug）。
6. 右键点击 → `UiEvent::MouseDown { button: Right }` 真的触发；
   中键点击 → `button: Middle`。Alt+键 → 不产生系统提示音（`WM_SYSCHAR` 已吞掉）。

前两条的"接线"由结构测试 `options_theme_reaches_backend` /
`options_font_reaches_text_engine` 守着（`grep` 源码断言，防止链路被拆），
但结构测试**不能**证明"窗口真的变亮/变大"——那一层必须有设备。
## §28 `InvalidationHandle`：后台线程如何触发重绘

**它是通知，不是状态容器。** 后台线程拿到
`let handle = tree.invalidation_handle();` 后，只能做两件事：

```rust
handle.invalidate("file_list");  // 下一帧标脏该节点
handle.invalidate_all();         // 下一帧整窗重绘
```

**不能**借它改组件字段——句柄里没有 `UiTree`、没有任何组件引用（这是它能
`Send + Sync` 的前提）。跨线程数据必须由应用自己管理：

```rust
let rows = Arc::new(Mutex::new(Vec::new()));
let handle = tree.invalidation_handle();
std::thread::spawn(move || {
    let fetched = fetch();                    // 后台干活
    *rows.lock().unwrap() = fetched;          // 应用自己的锁
    handle.invalidate("file_list");           // 只通知"该重绘了"
});
// UI 线程：Host::idle 里读 rows，写进组件；框架在 layout 前自动 drain 标脏请求
```

框架侧接线（主循环，`drain` 必须在 `layout` 之前）：

```rust
host.idle(&mut tree);
let external = tree.drain_external_invalidations();
if need_layout || external > 0 {
    window.resize(&mut tree);   // 内含 layout；尺寸不变时不重建交换链
}
```

要点：

- 与组件的分工：**组件走 `EventResult::Redraw`，异步任务走 `InvalidationHandle`**；
- 同帧多条请求合并：只要有一条 `invalidate_all()`，结果就是整窗脏区；
- `drain` 之后 `has_pending()` 归零；第二次 drain 返回 0；
- 句柄可以比树活得久（树销毁后 push 不会 panic，只是永远不会被 drain）。
### §28.1 保守策略：`One(id)` 也触发整窗重绘

`drain_external_invalidations` 对 `PendingInvalidation::One(id)` 也置
`DirtyRegion::full()`。这不是遗漏，是**有意的保守策略**：

- 后台任务改的多是内容（SFTP 列表更新、AI 消息追加），需要重排；
- 重排后节点位置可能变化，只标旧矩形会漏画新位置的节点；
- 整窗重绘是唯一能保证"重排后所有变化节点都被画到"的方式。

**代价**：后台每帧标脏 = 每帧全窗重绘。如果未来出现"只重绘不重排"的高频场景
（高频进度条、光标闪烁），可以引入 `PendingInvalidation::RedrawOnly(id)` 变体：
只标该节点矩形、不触发 layout。**当前不加**——没有真实场景证明它必需。

**应用层的责任**：后台更新频率极高时（例如每 16ms 一次），应用应自己节流
（例如 100ms 合并一次 `invalidate`），而不是依赖框架的脏区优化。
## §29 组件 `set_*` 之后的重绘责任

`Dialog::set_title` / `set_buttons` / `push_child` / `clear_children` / `set_open`，
`TextInput::set_text` / `clear` 等 `&mut self` 方法**不会自动标脏**——
组件碰不到 `UiTree`，这是分层原则（组件不知道自己被挂在哪棵树上）。

调用方（通常是 `Host`）在 `set_*` 之后必须自己补两件事：

1. `tree.mark_layout_dirty(id)` —— 尺寸/子节点可能变了；
2. `tree.invalidate(id)` —— 内容变了（或用 `invalidate_all` 保守处理）。

例外：**结构性变化**（`add_node` / `insert_node` / `remove_node`）由框架负责——
这三个方法会置 `layout_requested`，主循环下一帧自动 layout：

```rust
let structural = tree.take_layout_request();
if need_layout || external > 0 || structural {
    window.resize(&mut tree);   // 内含 layout
}
```

**为什么这样分工**：`add_node` 是框架 API，框架当然知道"树变了"；
而 `dialog.set_title(..)` 只是组件自己的字段赋值，框架无从得知——
让组件去反向持有树会破坏分层（组件之间、组件与树之间都靠事件通信）。
### §29.1 `set_open` 与其他 `set_*` 的标脏区别

| 方法 | 影响面 | 调用方必须 |
|---|---|---|
| `Dialog::set_title` | 只有该 Dialog 的标题文字 | `invalidate(id)` |
| `Dialog::set_buttons` | 该 Dialog 的按钮条（布局变化） | `mark_layout_dirty(id)` + `invalidate(id)` |
| `Dialog::set_open` | **全局**：`paint_order` / `modal_root` / `focus_order` | `mark_layout_dirty(id)` + **`invalidate_all()`** |
| `TextInput::set_text` | 该节点内容（尺寸可能变） | `mark_layout_dirty(id)` + `invalidate(id)` |
| `TextLabel::set_text` | 该节点内容（尺寸可能变） | `mark_layout_dirty(id)` + `invalidate(id)` |

为什么 `set_open` 特殊：`is_open() == false` 的子树在**四个地方**被跳过——
`rebuild_indices`（不绘制、不命中）、`render_pass`（不画）、`cursor_at`（不改光标）、
`rebuild_focus_order`（不进 Tab 遍历）。前三个跳过的是"这块矩形"，
第四个跳过的是"这张全局焦点表"，所以只 `invalidate(id)` 不够。
### §29.2 `set_focus` 的语义条件

`set_focus(id)` 返回 `true` 当且仅当四条同时成立：

1. 节点存在（`find_path` 能解析）；
2. 在 Modal 作用域内（`modal_root` 存在时，路径必须以其为前缀）；
3. 路径上**所有祖先（含自身）** `is_open() == true`；
4. 目标 `focusable() == true && !is_disabled()`。

条件 3、4 保证"关闭的子树不可聚焦"与 `rebuild_indices` / `render_pass` /
`cursor_at` / `rebuild_focus_order` 完全一致——五个地方用同一条规则。

**为什么不用 `focus_order.contains(&path)`**：`main.rs` 在**首次 layout 之前**就调
`set_focus(Some("title"))`，那时 `focus_order` 还是空表。用 `focus_order` 判定会把
合法的初始聚焦也拒掉。`path_is_open` 只看树结构，与是否已完成布局无关。

**注意**：条件 4 意味着"程序化聚焦"也只能落在可聚焦节点上。
如果某个节点需要被代码聚焦但不想进 Tab 序列，给它 `focusable() == true` 即可
（`focusable` 决定"能否聚焦"，Tab 序列还额外要求 `!is_disabled()`）。
**已知语义耦合**：`focusable()` 同时决定"能否进 Tab 序列"和"能否被 `set_focus` 接受"。
如果将来出现"代码聚焦但不进 Tab"的需求（比如模态内自动聚焦一个不可 Tab 的节点），
需要引入 `programmatically_focusable()` 之类的第二开关。当前没有这种需求，所以不拆。
## §30 密码输入的安全契约

`TextInput::password(true)` 时，`semantics()` 的**所有文本字段必须 mask**，
且 `sensitive = true`：

- `label`：`"password"`（不显示原文）；
- `value`：`"*".repeat(chars().count())`（按**字符数**，不是字节数——"中"是 1 个 `*`）；
- `state("text")`：同 `value`；
- `sensitive`：`true`；
- `cursor` / `anchor` / `len`：**保留**（长度已通过 `*` 的数暴露，这是有意接受的）；
- `text()`：**仍返回原文**（Host 提交给 SSH / 保存时用）。

**为什么是硬需求**：Agent 通过 `get_tree` / `query` / `subscribe` 读取 `semantics`。
不 mask 就等于把密码明文交给 AI。这不是便利性，是安全边界。

**渲染约束**：

- 正文每个字符（**含空格**）画 `*`——空格也要画，否则长度不可见；
- IME 预编辑串同样 mask（否则中文输入法会把明文画在光标处）；
- 光标 / 选区 / 点击位置仍按**原文本坐标**计算，交互完全不变。

**复制粘贴**：密码框的 `Ctrl+C` / `Ctrl+V` **保持正常**——用户从密码管理器
粘贴密码是常见操作，禁止粘贴会让密码管理器不可用。密码框的语义是"隐藏显示"，
不是"禁止复制"。应用如果想禁止，自己在应用层拦 `Ctrl+C`，不是框架的事。

## §30.1 占位提示（placeholder）

`TextInput::placeholder` 只在 `buffer.text().is_empty() && preedit.is_empty()` 时渲染；
**不进 `TextBuffer`、不进 `semantics`、不参与布局**。

当前颜色固定为暗灰（`rgba(0.55, 0.58, 0.64, 1.0)`），不跟主题走。

## TODO（框架侧，等需求触发）

- 主题加 `placeholder_fg` 字段，让占位提示跟随主题（当前固定暗灰）。
- `Dialog::set_size` 之后调用方必须 `mark_layout_dirty(id)` + `invalidate(id)`；
  `set_size` 的 doc comment 与 §29 都写明了，但容易被忽略——若将来出现
  "调用方忘了标脏"的实际问题，考虑让 `set_size` 走 `Command` 路径返回 `Relayout`。
## §31 Button 的选中态

`Button::selected(bool)` 用于"按钮组单选"场景（主题切换、Agent/QA 模式、SSH 类型等）。

- **渲染状态优先级**：`Disabled > Selected > Pressed > Hovered > Normal`。
  `Selected` 是**持久**状态（"当前选中的是哪个"），必须始终可见；
  `Pressed` / `Hovered` 是瞬时反馈，不覆盖它。
- **`Selected` 的配色**：没有独立主题列，由该角色的 `Normal` 向 `Pressed`
  混色 55% 派生（`Style::blend_toward`）——比 Normal 深、比 Pressed 浅。
  这样所有角色自动拥有选中态，不必给两张主题表各加 6 列。
- **`set_selected` 不自动标脏**：组件碰不到 `UiTree`，调用方必须 `tree.invalidate(id)`。
- **语义**：`semantics().state["selected"]` 是布尔值，Agent 据此判断
  "当前是暗色主题 / 当前是 QA 模式"。
- **运行时改选中态的唯一路径**：`tree.find_mut(id).as_any_mut().downcast_mut::<Button>()`
  （`Button` 已实现 `as_any_mut`）。

典型用法（三选一）：

```rust
// 构建
Button::new("btn-dark", "暗").selected(theme == Theme::Dark)
// 切换（Host 里）
for (id, on) in [("btn-dark", dark), ("btn-light", light), ("btn-system", sys)] {
    if let Some(b) = tree.find_mut(id).and_then(|w| w.as_any_mut())
        .and_then(|a| a.downcast_mut::<Button>()) {
        b.set_selected(on);
    }
    tree.invalidate(id);
}
```
## §32 组件作者守则：什么时候必须实现 `as_any_mut`

`Widget::as_any_mut` 是运行时 downcast 的**唯一**路径：

```rust
tree.find_mut(id)?
    .as_any_mut()?
    .downcast_mut::<MyWidget>()?
```

**规则**：如果你的组件有 `&mut self` 的运行时方法（`set_text` / `set_open` /
`set_selected` / `push_child` / `write` / `pump` 等），**必须**实现 `as_any_mut`——
否则 Host / Agent 无法在运行时调用这些方法，功能**静默失效**（不报错、不 panic，
只是什么都不发生）。

**反例（不需要 `as_any_mut`）**：

- 只有构建器（`fn x(mut self) -> Self`）——构建期调用，不需要运行时 downcast；
- 只有 `&self` 只读方法——`find(id)` 就够；
- 内部组件（如 `ListView` 的 `Row`）——由父容器自己 downcast。

**已实现 `as_any_mut` 的组件（截至 §32 编写时）**：

| 组件 | 运行时方法 |
|---|---|
| `Button` | `set_selected` |
| `TextInput` | `set_text` / `set_password` / `clear` |
| `TextLabel` | `set_text` / `refresh` |
| `Dialog` | `set_title` / `set_buttons` / `set_open` / `set_size` / `clear_children` / `push_child` / `close` |
| `ListView` | `set_items` |
| `Menu` | `open` / `close` |
| `TerminalWidget` | `write` / `pump` / `attach_shell` / `grid_mut` |
| `Row`（内部） | 行复用（父容器调用） |

**为什么不在 `Widget` trait 里提供默认实现**：`as_any_mut` 返回 `&mut dyn Any`，
而 `dyn Any` 必须指向**具体类型**（`Self`）——默认实现在 `dyn Widget` 上无法返回
`Self`（`Self: Sized` 之外的类型擦除限制）。这是 dyn 类型系统的限制，
所以每个需要运行时 downcast 的组件必须显式实现。

**反模式**：不要为了"能 downcast"而给只读组件补 `as_any_mut`——只读用 `find(id)`。
另外 `Flex` **刻意不实现** `as_any_mut`：运行时加子节点的正确路径是
`tree.add_node(flex_id, child)`（会 `on_mount`、查重、标脏、置 layout 请求）；
用 downcast 直接 `push_boxed` 会跳过挂载，是坑。
## §33 黑屏 bug：没有脏区时不能 Present

**现象**：窗口启动后全黑，点一下鼠标闪出一帧内容，随即又黑。

**根因**（`Window::render` + `UiTree::render` 的组合）：

1. runtime 每帧都调 `window.render()`；
2. `UiTree::render` 在 `dirty.is_empty()` 时**直接返回**（不产生任何实例）；
3. 但 `Window::render` 无条件 `backend.begin()`（清空实例）→ `present()`；
4. 于是"零实例"的帧被 Present 出来 = 清屏成背景色；下次输入产生脏区才画一帧，
   下一帧又被清掉。表现为"闪一下又黑"。

**修法**：

```rust
pub fn render(&mut self, tree: &mut UiTree) {
    if tree.dirty().is_empty() { return; }   // 不清屏、不 Present，保留上一帧
    …
}
```

**配套：遮挡 / 最小化恢复**。D3D 交换链的内容**不受** Windows 保护，
遮挡或最小化恢复后客户区是未定义的（黑或花）。而"有脏区才画"意味着此时通常
没有脏区，所以必须显式请求一次全量：

- `WM_PAINT` → `request_full_redraw()`（置一次性标志，只 Begin/EndPaint 不够）；
- runtime 每帧 `if window.take_repaint_request() { tree.invalidate_all(); }`
  （必须在 `window.render` **之前**）。

**为什么不改成"每帧都重画"**：那是纯浪费（空转 CPU/GPU），而且仍然会在
`present` 失败或设备丢失时闪；"保留上一帧"才是正确的呈现模型。
## §34 卡顿与裁剪：主循环空转 + 客户区裁剪缺失

### 34.1 主循环空转（极度卡顿的真凶）

`Win32Runtime::run` 原本是：

```rust
loop {
    if !window.pump() { break }   // PeekMessage：**非阻塞**
    … 处理事件 / 布局 / render …
}
```

`PeekMessage` 没有消息时立刻返回，于是"没有输入、没有脏区"时这个循环
**全速空转**吃满一个核——表现为鼠标移动/点击有明显延迟（Windows 输入线程抢不到 CPU）。

修法：空闲时阻塞等待消息（有限超时）：

```rust
if !had_work && tree.dirty().is_empty() {
    let _ = window.wait_message(8);   // MsgWaitForMultipleObjectsEx(QS_ALLINPUT, 8ms)
}
```

- **不能无限等**：PTY 输出走 channel 而不是 Windows 消息，必须周期性醒来 `pump`；
- 8ms ≈ 125 次/秒的空转上限，相比 100% 占满已可忽略。

### 34.2 客户区裁剪缺失（文本画到标题栏外）

两个独立缺口：

1. `D3dBackend::begin()` 只 `clips.clear()`，**没有基座裁剪**——
   于是 `clip_of()` 在 `clips` 为空时原样返回矩形，什么都没裁。
2. `push_glyph()` **完全不裁剪**：`clip_of` 只被 `fill_rect` 调用，
   字形实例是无条件 push 的。

修法：

1. `begin()` 压入客户区作为基座裁剪：`clips.push(Rect::new(0, 0, viewport.w, viewport.h))`；
2. 字形按裁剪框缩放 + 重映射 UV（`clip_quad`）：完全在框外直接丢弃，
   部分越界则裁剪矩形并按比例重算 UV——比整字形丢弃更平滑（不会出现半截字突然消失）。

**GPU scissor 已经上了**（见 §36）：`RSSetScissorRects` vtable 槽已补齐（按
`winapi::um::d3d11` 的 `ID3D11DeviceContextVtbl` 逐伪对齐），`scissor_enable = 1`，
局部重绘按**脏区列表**逐块清屏 + 逐块画。CPU 侧 `clip_quad` 作为
字形裁剪的保险依然保留（两道防线）。

### 34.3 仍存在的性能隐患（未修）

`finish_dispatch` 与 `dispatch` 的通用分支：**任何 `Relayout` 都会把脏区置为 full**，
而 `UiEvent::Tick` 是广播事件（`target_id == None`），组件返回 `Redraw` 时也会
`invalidate_all()`。如果终端光标闪烁或 PTY 轮询每帧触发一次 Tick，就会每帧全量重绘。
当前没有把它改细，因为"重排后位置可能变、只标旧矩形会漏画"（§28.1 同样的理由）。
若真出现掉帧，可以考虑：`Tick` 的 Redraw 收集各响应节点的矩形后逐块标脏。
## §35 广播事件的重绘粒度 + 帧耗时打点

### 35.1 Tick 曾经导致整窗重绘

`Host::idle` 每帧派发 `UiEvent::Tick`（最多 8 轮）。`dispatch` 的通用分支是：

```rust
} else if result.needs_redraw() {
    match target_id {
        Some(id) => self.invalidate(id),
        None => self.invalidate_all(),   // ← Tick 是广播，永远走这里
    }
}
```

于是"终端每帧从 PTY 读到数据" → `Redraw` → **整窗重绘**。窗口越大越慢。

修法：`broadcast()` 在遍历时收集**真的返回了 `Redraw`** 的节点矩形
（`UiTree::broadcast_rects`，复用缓冲不分配），`dispatch` 的 `None` 分支改为
逐块 `dirty.add(rect)`；只有收集为空时才退回 `invalidate_all()`。

### 35.2 帧耗时打点（`Y901_PROFILE=1`）

`Win32Runtime::run` 内置统计，每 2 秒向 stderr 打一行：

```
[profile] frames=431 rendered=57 full=57 partial=0 avg=317us max=2531us
```

- `frames`：2 秒内循环次数（≈ 上限 125/s，受 8ms 休眠限制）；
- `rendered`：真正执行了 `present` 的帧数（0 = 完全空闲）；
- `full` / `partial`：整窗重绘 vs 局部重绘；
- `avg` / `max`：单帧耗时（从 `pump` 到 `present` 返回）。

默认关闭，开启时只有几次 `Instant::now()` 的开销。

实测（本机虚拟显卡，release）：

| 场景 | CPU（占单核） | 平均帧 | 最大帧 |
|---|---|---|---|
| 空闲 | **0.17%** | 10–104us | 2.5ms |
| 连续拉伸窗口（60 次/2s） | **6.3%** | 317us | 5.7ms |

两者都远低于 16ms 预算。

## §37 脏区列表化：多块不相邻脏区不能合并成包围盒

**问题**：`Window::render` 曾经只把 `dirty.bounds()`（单个包围盒）传给后端。
两块不相邻的脏区合并后，**中间的空隙也被清屏**；而 CPU 侧剪枝
（`needs_paint`）已经跳过了不与任何脏区相交的节点 → 空隙里的内容被清掉且不重绘 → **变黑**。

**修法**：

1. `Window::render` 传 `Vec<(i32,i32,i32,i32)>`（逐脏区）而非包围盒；
2. `Gfx` 保存脏区列表，`render` 里**逐块**执行 `scissor + clear + DrawInstanced`；
3. 脏区数 > `MAX_DIRTY_RECTS (16)` 时降级为整窗重绘（碎片化时 draw 次数翻倍反而更慢）。

**验证**：`two_disjoint_dirty_rects_match_full_redraw`（GPU，`#[ignore]`）
—— 左上、中间（**不受脏区覆盖**）、右下三个独立子控件，
只标左上与右下两块 → 与整窗重绘逐字节相等。
变异检验：改回包围盒方案 → 测试立刻在 `(200,80)`（中间块）报错。
