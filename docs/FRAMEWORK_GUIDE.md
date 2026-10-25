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

以下问题已识别但**未修复**，留待后续轮次处理（不在当前改动范围内）：

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
## §16 TODO：把 RPC 回包移出 UI 线程（跨平台 v3 之后做）

**现状**：`reply_to` 在 UI 线程内同步写 socket，靠 `set_write_timeout(100ms)`
把最坏阻塞压到 100ms；写失败/超时立即 `shutdown` 断开连接。

**为什么还不够**：UI 线程阻塞 100ms 就是一帧掉 6 帧（60fps）。
正常路径不会触发，但对"性能至上"的框架来说，这是一笔不该存在的风险。

**彻底方案**：每连接一个写线程 + 统一 FIFO 队列。
回包与推送都进同一个队列，由写线程按入队顺序写出。
收益：UI 线程**永不**碰 socket（零阻塞），且顺序由队列天然保证（§15 的顺序契约）。

**迁移步骤**
1. `Pending` 携带该连接的 `Sender<String>`，`reply_to` 改为 `send`（非阻塞）。
2. `Broadcaster` 从"持 socket"改为"持每条连接的 Sender"。
3. 每连接一个写线程：`for line in rx { write_all }`，写失败即断开并注销。
4. 删除 `set_write_timeout`（写线程阻塞无害，UI 不受影响）。
5. 测试：`rpc_write_timeout.rs` 改为断言"UI 线程零阻塞"，而不是"100ms 内返回"。
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