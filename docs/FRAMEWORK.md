# Y901 框架总览

Y901 是一个**零 C/C++ 编译依赖**的桌面 UI 框架。UI 框架本身不认识任何平台 API，
平台能力（Win32 / D3D11 / DirectWrite / PTY）全部收敛在应用层。

## 1. 十一个 crate

| crate | 职责 | 生产依赖 |
|---|---|---|
| `y901-core` | 纯逻辑：终端 Grid / reflow / scrollback / 格宽（`width_of_char`） | `unicode-width` |
| `y901-ui` | 框架：Widget / UiTree / 布局 / 事件 / Signal / 语义树 / revision | `y901-core`（内部，仅布局原语 `IntervalSet`） |
| `y901-widgets` | **通用**组件：Button / TextLabel / TextInput / ScrollView / ListView / SplitView / Menu / Dialog / StyledText | `core`, `ui` |
| `y901-extras` | **扩展**组件：TerminalWidget（Grid + VT + PTY） | `core`, `ui`, `crossbeam-channel`, `portable-pty` |
| `y901-runtime` | 平台无关契约：`Host` / `PlatformRuntime` / `RawEvent` / `translate` / `RenderBackend` | `ui` |
| `y901-backend-win32` | 唯一接触 Win32 的 crate：窗口 / D3D11 / DirectWrite / IME / DPI | `core`, `ui`, `runtime`, `windows-sys` |
| `y901-app` | 启动器：解析参数、建树、装配 Host、跑事件循环（不实现组件、不定义布局） | `backend-win32`, `runtime`, `ui`, `widgets` |
| `y901-render` | 软件光栅化器（SoftBackend）与自研 TTF 字形引擎（图集 / mipmap LOD） | `ui`, `ttf-parser` |
| `y901-shell` | 应用壳：菜单栏 / 托盘 / 快捷键绑定 | `ui`, `runtime`, `windows-sys`（命令行 / 磁盘操作）|
| `y901-backend-x11` | Linux X11 后端（Xvfb 下验证，真机待验） | `core`, `ui`, `render`, `runtime`, `x11rb` |

依赖方向严格单向（箭头指向被依赖方）：

```
y901-app ──> y901-backend-win32 ──> y901-runtime ──> y901-ui
    │                │                    │                │
    │                └──> y901-core <─────┴──> y901-widgets ┘
    └──> y901-widgets / y901-ui
```

关键约束：

- `y901-ui` **零外部依赖**：它只依赖内部 crate `y901-core`（用于布局原语 `IntervalSet`）。
  这不是洁癖，而是测试策略的地基：
  751 项测试（另有 7 项 GPU harness，`--ignored` 运行）里绝大多数不需要窗口、GPU、PTY。
- **`y901-backend-win32` 与 `y901-shell` 都直接依赖 `windows-sys`**（grep 可验证；前者用于窗口/渲染，后者用于命令行与磁盘操作）。
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
    // 以下都有默认实现；D3D11 后端覆盖为 GPU 图元（SDF / 渐变）
    fn fill_round_rect(&mut self, rect, color, radius);
    fn stroke_round_rect(&mut self, rect, color, width, radius);
    fn fill_gradient_rect(&mut self, rect, from, to, horizontal);
    fn fill_gradient_rect_dir(&mut self, rect, from, to, dir: (f32, f32));
    fn fill_gradient_round_rect(&mut self, rect, from, to, dir: (f32, f32), radius);
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
    fn attach_waker(&mut self, waker: Arc<dyn Waker>);                     // 默认空实现
    fn policy(&self) -> &Policy;
    fn ime_position(&self) -> Option<Pos>;
}

/// 跨线程唤醒器：后台线程（PTY/RPC）拿到数据时 `wake()`，
/// 打断主循环的阻塞等待（否则要等下一次输入）。
pub trait Waker: Send + Sync + 'static {
    fn wake(&self);
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

## 9. 语义树与查询（网关已移除）

去 AI 阶段把 `y901-agent` / `y901-mcp` 从工作区物理删除：RPC、MCP、Token / 作用域 / 脱敏那一层不再存在。
保留在 `y901-ui` 内的是**语义树**本身——`Semantics`（label / value / state / actions / role）、
`query` 选择器、有效 revision 与语义面哈希。它们服务的是"外部工具/自动化读界面"，
默认 `exposed_to_ai = false`，组件必须显式 `.exposed(true)` 才会出现在语义输出里。

## 10. 测试

```
cargo test --workspace          # 712 项（另 7 个 GPU harness 需 --ignored）
cargo build --workspace         # 零警告
cargo clippy --workspace --all-targets   # 警告必须为 0（建议直接用 scripts/gate.ps1） 零警告
scripts/test-count.ps1          # 唯一可信的计数方式（把输出写文件再解析）
```

绝大多数测试无需窗口/GPU：布局、事件路由、裁剪命中、语义树、`query` 选择器、
图集打包、revision 语义面哈希都能在 CI 里跑。

需要真机的测试只有 `rebuild_reuploads_atlas`（D3D11 设备重建后图集重传），
在无显卡权限的机器上会打印 `[skip]` 并返回。

## 12. Overlay（浮层）机制

任何控件都可以把自己挂到**屏幕最上层**，不受父级 `push_clip` 裁切：

```rust
// 挂载（anchor = 触发控件的屏幕矩形）
tree.open_popup(Box::new(my_menu), trigger_rect);

// 查询 / 关闭
tree.popup_rect("my_menu");   // 布局后的屏幕矩形（绘制与命中同源）
tree.close_popup("my_menu");
tree.close_all_popups();
tree.has_popup();
```

不变量：

1. **裁切只到视口**：浮层在所有层之后绘制，clip 只压视口；
2. **摆放不越界**：用 `place_popup(anchor, size, viewport)`，
   下方放不下向上翻转、右侧放不下向左平移；
3. **事件最高优先级**：命中浮层 → 直接派给它；`MouseDown` 在外部 → 关闭全部浮层并消耗；
4. **关闭必标脏**：`close_popup` / `close_all_popups` 会把浮层原位置加进脏区。

### 12.1 在事件里弹出（推荐）

组件拿不到 `UiTree`，所以框架提供**请求队列**：事件里提交工厂，
分发结束后由 `finish_dispatch` 统一构造 + 挂载。

```rust
impl Widget for FileTable {
    fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult {
        if let UiEvent::MouseDown { pos, .. } = e {
            if self.hit_more_button(*pos) {
                let rows = self.rows.clone();
                ctx.request_popup(
                    Box::new(move || Box::new(MoreMenu::new("more", rows))),
                    self.more_button_rect(),
                );
                return EventResult::Handled;
            }
        }
        EventResult::Ignored
    }
}
```

- `ctx.request_popup(factory, anchor)`——普通浮层（点外部自动关）；
- `ctx.request_modal_popup(factory, anchor)`——模态（叠黑遮罩、底层不接收事件、
  点外部**不**关闭）；
- 工厂只调一次；弹出失败（id 重复）时工厂不会被调用？——**会被调用**，
  但 `open_popup` 返回 `false` 并丢弃构造出的 widget；调用方无需关心。

### 12.2 唯一机制的架构决定

> **所有浮层（Popup / Dialog / Menu）统一走 `request_popup` / `request_modal_popup`。**
> `Layer` 枚举与 `deferred` 回放已彻底删除。

| 能力 | 现在的唯一机制 |
|---|---|
| 挂载 | `tree.open_popup` / `open_modal_popup`，或组件在事件里 `ctx.request_popup` |
| 裁切 | 只到视口（天然不受父级影响） |
| 点外部关闭 | 非模态自动；模态只吃事件不关闭 |
| 内容自关 | 浯层内容 `is_open() == false` → 浯层层自动撞掉 |
| 命中/绘制一致 | 同一个 `rect`（`place_popup` 结果） |
| 语义树 | 已合并进 `snapshot()` |
| 焦点作用域 | 模态时 Tab 只在浯层内循环，关闭后恢复原焦点 |

`Layer` 与 `deferred` 回放已删除：绘制顺序就是**树序**（父先子后、兄弟按序），
“画在所有东西之上”由 `UiTree::popups`（树外、最后画）提供。

### 12.3 模态浮层的焦点作用域（已实现）

`open_modal_popup` / `request_modal_popup` 打开后：

| 事件 | 行为 |
|---|---|
| `Tab` / `Shift+Tab` | **只在浮层内的可聚焦控件之间循环**（`focus_next_in_modal`），绝不逃回底层 |
| `Escape` | 关闭全部浯层并恢复焦点 |
| 其它键盘 / 文本 | 仅当焦点属于浯层时派发；否则丢弃 |
| 鼠标 | 命中浯层 → 派给它；点外部 → 吃掉事件，不关闭 |
| 关闭后 | 焦点还给弹出前的控件（`Popup::prev_focus`） |

实现要点：`set_focus` 对浯层内的 id 走独立路径（它们不在树里，
没有 `find_path`），且不受 `modal_root` 限制。

### 12.4 层与模态彻底解耦（已完成）

- **`Layer` 枚举、`Widget::layer()`、`layer_present`、`deferred` 回放全部删除**；
- 模态作用域由 `Widget::is_modal()` 宣言（`Dialog` 返回 `self.open`）；
- 绘制顺序 = **树序**；“画在所有东西之上”交给 `UiTree::popups`（树外、最后画）。

这是一次语义分离：之前“画在哪层”顺带决定了“是否阻塞底层”，
导致想要“画在上层但不阻塞”（Tooltip、右键菜单）无法表达。

### 12.5 迁移后的心智模型

| 你想要 | 用什么 |
|---|---|
| 一个普通面板/列表 | 直接当子节点（树内） |
| 一个点外部就关的菜单/Tooltip | `ctx.request_popup(...)` |
| 一个阻塞底层的对话框 | `ctx.request_modal_popup(...)`（`Dialog` 记得 `set_scrim(false)`） |
| “画在最上层但不阻塞” | 非模态 popup |

## 13. RichText（富文本）接口

框架只提供“数据 + 排版”，不提供业务语义（不知道什么叫“流式追加”）：

```rust
pub struct RichText;                     // 组件（Widget）
pub enum Block { Paragraph(Vec<Span>), Code { lang: String, lines: Vec<String> } }
pub struct Span { text: String, style: InlineStyle }
pub struct InlineStyle { bold, italic, color: Option<Color> }

impl RichText {
    pub fn update(&mut self, blocks: Vec<Block>);   // 核心 API
    pub fn blocks(&self) -> &[Block];
    pub fn content_height(&self) -> f32;            // 最近一次 measure 的结果
    pub fn block_rect(&self, i: usize) -> Option<Rect>;      // 屏幕矩形
    pub fn last_block_rect(&self) -> Option<Rect>;           // 配合 invalidate_rect 做局部标脏
}
```

流式追加由**调用方**完成：改最后一个 `Block` 的文本 → `update(blocks)` →
`last_block_rect()` → `tree.invalidate_rect(rect)`。框架负责重排与只重绘那一块。

## 14. 通用化边界：什么进框架，什么不进

判据标准只有一条：**它是机制，还是某个业务的策略？**

| 能力 | 归属 | 理由 |
|---|---|---|
| 布局（Flex/Stack）、事件路由、焦点、能力集 | `y901-ui` | 任何 GUI 都需要 |
| 滚动、按钮、文本输入、列表、菜单、对话框 | `y901-widgets` | 通用控件，不依赖子系统 |
| 终端（Grid + VT 序列 + PTY） | **`y901-extras`** | 依赖进程/伪终端，是子系统策略 |
| 文件拖放事件 `UiEvent::FileDrop` | `y901-ui` | OS 级能力，不属于任何业务 |
| Markdown 解析 | 应用层 | 语法是业务 |
| 流式追加文本 | 应用层 | 框架只需 `StyledText::update(Vec<Block>)` |

### 14.1 响应式更新：三种途径，一个结果

| 你在哪里 | 用什么 | 脏区 |
|---|---|---|
| 组件的 `event()` | `ctx.request_redraw()` / `ctx.request_relayout()` | 自动（命中节点矩形） |
| 非事件途径（后台/定时器/回调） | `tree.use_state(id, v)` → `state.set(v)` | 自动（订阅者所在节点） |
| 跨线程 | `InvalidationHandle::invalidate(id)` | 自动下一帧 |

三条路最后都汇到 `drain_invalidation()`：它只把**相关节点的矩形**加进脏区，
拿不到矩形才退化整窗。（早期实现无条件 `dirty = full()`，把响应式变成了整窗重绘。）

## 15. ListView：通用虚拟列表（无数据）

早期的 `ListView` 存 `RowData { cells, icon }`——那是"文件表格"一个业务的形状。
聊天气泡、图片流、日志行都装不进去。

现在它只管三件事：**总行数、当前偏移、可见区裁剪**；行长什么样由回调决定。

```rust
let list = ListView::new("chat", 24.0)
    .len(messages.len())
    .stick_to_bottom(true)
    .painter(move |i, rect, ctx| {
        // 自己画：气泡、头像、图片…都行
        ctx.draw_text(Pos::new(rect.x + 8.0, rect.y + 4.0), &messages[i].text, &style);
    });
```

| API | 作用 |
|---|---|
| `len(n)` / `set_len(n)` | 总行数（\`set_len\` 返回是否变化） |
| `painter(f)` / `set_painter(f)` | 行绘制回调 `FnMut(usize, Rect, &mut dyn RenderCtx)` |
| `visible_range()` | 当前可见区间 |
| `row_rect(i)` / `invalidate_row(i)` | 单行屏幕矩形（已裁到可见区） |
| `stick_to_bottom` / `scroll_to_end` / `is_at_bottom` | 粘底（日志/聊天） |
| `scroll_to_row(i)` | 滚到指定行 |
| `from_strings(id, items)` | 便利构造：纯文本行（数据被捕获在回调里，**不进**字段） |
| `row_at(pos)` | 屏幕坐标命中的行下标（已考虚滚动） |
| `set_row_height_provider(f)` | 动态行高（聊天气泡/折叠行）；不设则用固定值 |
| `row_height_at(i)` | 某行高度（至少 1px） |
| `clear_row_height_provider()` | 回到固定行高 |

动态行高的实现：内部维护 `prefix[i] = 前 i 行高度和`，
`row_rect`/`row_at`/`visible_range`/`scroll_to_row` 全部走二分查找
（O(log n)）。提供者变更（`set_row_height_provider`）会作废前缀和，
下一次 `measure`/`arrange` 重算。

行**不再是子节点**：`children()` 为空，命中/标脏用 `row_rect`。
代价是每行没有独立的语义节点；因此 `semantics()` 只报
`rows/offset/first_visible/selected` 四个属性。若需逐行语义，用普通容器 + 子控件。

## 已修复：`cell_span` 的格数已统一（曾有 5 处 `.max(1)` 违反契约）

- **格数的唯一来源**：`y901_core::Grid::width_of_char`
  （日志：0 → 组合符 0 格，窄 → 1，宽/全角 → 2；控制字符由 `unwrap_or(1)` 兵底为 1）。
- **两后端的 `cell_span` 本就同源**（均直接调 `width_of_char`）：
  - soft：`crates/y901-render/src/font_engine/engine.rs:121`
  - D3D：`crates/y901-backend-win32/src/backend/font.rs:1026`
- **曾存在的问题（已修）**：6 个**消费点**里有 **5 个**对结果做了 `.max(1)`二次加工 ——
  这使组合字符 / 零宽字符被当成 1 格，与 `width_of_char` 的 "0 格"直接冲突：
  | 位置 | 函数 | 处理 |
  |---|---|---|
  | `soft.rs` measure_text | 度量 | 已删 `.max(1)` |
  | `soft.rs` draw_text | 绘制推进 | 已删 |
  | `soft.rs` draw_text_sized | 绘制推进 | 已删 |
  | `backend/mod.rs` draw_text | 绘制推进 | 已删 |
  | `backend/mod.rs` push_glyph | 绘制推进 | 已删 |
  | **`backend/mod.rs` measure_text** | 度量 | **本来就没有**（反而是对的） |
- **现在**：6 个消费点**全部直接用 `cell_span(ch)`**，无二次加工 → **两后端对同一文本的度量完全一致**。
- **`span = 0` 的行为**：**仍绘制字形，但推进 0** —— 组合字符（U+0301）必须叠在前一个字上。
- **测试守护**：
  - `y901-core`：`width_of_char_tests`（5 条：ASCII/wide/combining/zero-width/control）；
  - `y901-render`：`tests/cell_span_semantics.rs`（4 条，含 **组合字符不增宽** 与 **零宽字符宽度 = 0**）。
- **影响**：`hash-gate` **未变**（`0xDBB308DD1657EF9C`）—— 场景文本不含组合/零宽/控制字符，
  而常规字符的 `.max(1)` 本就是无效操作。

## 已支持：`TextStyle.size` 的 per-text 字号（soft 与 D3D 两后端均已实现）

- **并非未实现**：
  - `TextMetrics::measure_text`（`soft.rs:316` / `backend/mod.rs:654`）**读 `style.size`**；
  - `TextMetrics::line_height`（`soft.rs:323` / `backend/mod.rs:659`）**读 `style.size`**；
  - **但 `RenderCtx::draw_text`（`soft.rs:646` / `backend/mod.rs:962`）不读** ——
    两处直接用 `self.cell`（引擎的唯一字号 cell）。
- **后果**：`TextStyle::size(px)`、`TextLabel::size(px)`、`InlineStyle::size(px)`
  **只会改变布局占位**；字仍然按引擎默认字号绘制。
- **原因**：引擎只光栅化一套字号的字形（`MyFontEngine::new(data, size_px, cell)`）。
  按任意 `size` 绘制需要"多字号光栅化"（每个字号都有真实字形，而非把默认字号拉伸）。
- **影响（下游应用）**：此前设置的分组标题 18px / 表头 13px 在**视觉上未生效** ——
  分组标题仍按 16px 画（只是布局预留了 18px 高）；
  更麻烦的是**表头预留了 13px 高却按 16px 画 → 文字可能被上下裁掉**。
  （`bold` 是**真的**：`draw_text` 读 `style.bold`，所以加粗那部分视觉生效。）
- **修复方向**：多字号光栅化（`draw_text` 按 `style.size` 取/生成字形）。
  已有前提：自研光栅化器 `rasterize_glyph_phased(data, ch, size, phase)`
  **本就接受字号参数**，图集缓存 key 已是 `(char, phase)` 元组 ——
  **加一个尺寸维度即可**，不需要新的字体轮廓访问路径。

### 为何选方案①（引擎内多字号）而不是方案②（后端多引擎）

- **方案②被否决的硬事实**：
  两个后端只持有 `text: Box<dyn FontEngine>`（`soft.rs:75`）——
  **拿不到具体的 `MyFontEngine`，也拿不到字体字节 `data: Vec<u8>`**。
  要支持“每字号一个引擎”就得：改后端构造契约（传字体字节）+ 后端加字节字段 +
  把 trait 对象改成同时持有具体类型 + 两个后端各复制一套管理 + 改 supersample/x11 调用点。
- **方案①的前提全部已就位**：`data: Vec<u8>` 就在引擎里、
  `rasterize_glyph_phased(data, ch, size, phase)` 本就接受字号、图集缓存 key 是元组（加一个维度即可）、
  驱逐/占用率/清空都在引擎内。
  **后端只需多调一个 `glyph_at_size(ch, px)`。**
- **已实现**：`cache` key `(char, phase)` → `(char, phase, px)`；`px_key()` = 引擎基准字号取整；
  新增 `MyFontEngine::glyph_at_size(ch, px)`；`FontEngine` trait 加默认实现（委托 `glyph`，
  所以默认字号路径**逐像素不变**）。
  `glyph_at_size` 对非基准字号**不生成 mip**（mip 的级尺寸按基准字号算，不适用于其他字号）。
- **待做**：两后端 `draw_text` 改用 `glyph_at_size` + `cell_metrics_for(px)`（尚未生效）。

#### 当前状态（中间态，待 D3D 补齐）

- **soft 后端：已支持 per-text 字号。**
  `draw_text` 在 `style.size` 为 `Some(px)` 时走 `draw_text_sized`：
  `px_final = px * map_scale()`，按该字号**重新光栅化**（`glyph_at_size`）并 1:1 绘制。
  `size: None` 仍走**原路径**（106 行逐字未动）。
- **D3D 后端：已支持。** `draw_text` 在 `style.size = Some(px)` 时转 `draw_text_sized`
 （`px_final = px * map_scale()`，按该字号重新光栅化 `glyph_at_size` 后 1:1 绘制）。
- **临时分裂**：**软渲染下** 18/13 字号生效；**D3D 下**仍按默认字号画。D3D 完成后分裂消失。
- **重要：多数下游应用主路径是 D3D** —— 用户当前看到的行为**与改动前一致**（D3D 不支持字号），
  **这不是回归**，而是"能力先落软渲染侧"。
- **待做**：D3D 中改 `TextEngine`（缓存 key 加字号 + `glyph_run_analysis` 接受 size + 覆写 `glyph_at_size`）+ `backend/mod.rs::draw_text` 同源分支。

#### 技术债：`push_glyph_sized` 的 `#[allow(clippy::too_many_arguments)]`

- `backend/mod.rs` 的 `push_glyph_sized` 有 **9 个参数**（`x, y, span, ch, color, bold, px, ascent, cell_w`），
  因此加了 `#[allow(clippy::too_many_arguments)]`。
- **这是风格 lint，不是正确性问题**：不影响编译、不影响行为、不影响 hash 基准。
- **为何不拆**：拆成 `GlyphRenderContext { px, ascent, cell_w, span, bold }` 会动到
  调用点与旧路径的对比可读性；且该方法只有一个调用点（`draw_text_sized`）。
- **后续建议**：若之后还要加参数（如 letter_spacing / 9 号以上），再打包为 struct。
  **当前不阻塞功能，记录为技术债。**

### DPI × scale：字号已接入（高度常量待做）

- **当前状态（已完成）**：
  - **基准字号**：`FontConfig.size × dpi_scale`（`window.rs` 用 `GetDpiForWindow(hwnd)/96.0`）。
  - **per-text 字号**：`Some(px)` 路径在两个后端同源乘 `self.dpi_scale`
    （`measure_text` / `line_height` / `draw_text_sized` 各 3 处）。
  - **`None` 路径不乘**：`self.cell` 已是 `FontConfig.size × dpi_scale` 的物理像素，再乘就是双重缩放。
  - `px_final = px × dpi_scale × map_scale()`：**两个 scale 来源不同**（设备 DPI vs 变换栈），不重复。
  - `TextMetrics::scale()` 已修正为返回**窗口级** DPI（字段注入）。
    **实测：`GetDpiForSystem()`=96 而窗口 DPI=120** —— PMv2 进程下不能用系统 DPI。
- **已知中间态缺陷（下一轮必须修）**：
  - **高度常量尚未乘 scale**（`TAB_BAR_H` / `TOOLBAR_H` / `ITEM_H` / `ROW_H` 等约 12 组、约 18 个常量）。
  - 125% 下**字从 16→20，但标签栏仍 32px** → **文字会顶边/裁切**。
  - 不随 scale 的约 10 个（`split::DIVIDER` 1px、`CARET_W`、`BAR_WIDTH`、`CHECK_W`、`RADIUS`等）**应保持不变**。
- **hash-gate 基线**：当前为 **`0x39AE41C1697CBFD0`**（门禁已固定 `Y901_TEST_DPI_SCALE=1.0`）。
- **曾经的错误归因（已更正）**：
  当时记录过 `0x87DECF871BB90AFF`，并写成"原因：基准字号 16→20 @125%"。
  **这个归因是错的**：
  - DPI=1.0 时算出的是 `0xDBB308DD1657EF9C`；
  - DPI=1.25 时算出的是 `0xFC83652FBBC7188B`；
  - `0x87DECF871BB90AFF` **两者都不是** —— 它是一个**不可复现的环境值**（当时运行会话的 DPI 与现在不同）。
  若真是 1.25，当年就应算出 `0xFC83…`，而不是 `0x87DE…`。
- **正确结论与完整实证见 [`docs/hash-gate-scope.md`](hash-gate-scope.md)**：
  门禁场景只渲染一个自绘 `Scene`（不含任何框架控件）；
  自 `75af5b1` 起字号乘 DPI 缩放 → 门禁曾失去可复现性，现已固定 DPI=1.0。

### `WM_DPICHANGED`：DPI 变化已实现（2026-11-12）

- **wndproc**（`window.rs`）新增 `WM_DPICHANGED` 分支：
  - 从 `wParam` 取新 DPI（`LOWORD`）写入 `WindowShared::pending_dpi`；
  - 按 `lParam` 的**系统建议矩形**调 `SetWindowPos`（`SWP_NOZORDER | SWP_NOACTIVATE`）——
    如果不调，窗口尺寸与新 DPI 不匹配，看起来像被裁切。
- **`Window::after_frame`**（每帧尾巴）消费该标记：
  - `take()` 后置 `None` → **只在真正变化的那一帧重建**，不会每帧重建；
  - 用 `base_font_size`（**未乘 scale 的用户配置字号**）× 新 scale 重建引擎；
  - `backend.set_dpi_scale(scale)` + `soft.set_dpi_scale(scale)`（`Some(px)` 路径依赖它）；
  - `resize(tree)` + `tree.invalidate_all()` → 全量重排与重绘。
- **为何不在 `wndproc` 里直接重建**：它是**自由函数**，拿不到 `Window`/`backend`/`tree`。
- **已知行为**：重建引擎会让字形图集**全空**，雀一帧需重新光栅化可见区文字 —— **一次性 1 帧代价**。
- **已知限制**：`wParam` 高低位不等时（各向异性 DPI）**取 `LOWORD` 统一处理**；Windows 当前不产生此情况。
- **失败处理**：`set_font` 返 `Err` 时打印错误并**保持旧引擎**（画面仍可用，只是字号未变），不 panic。
- **需用户手动验证**：将窗口从 100% 显示器拖到 125% 显示器再拖回去，
  观察：① 字号是否跟随变化；② 布局是否重排（无重叠/越界）；③ 是否有黑帧/闪烁。

### wrap（`Flex::wrap(true)`）—— 2026-11-12 实现

- **用法**：`Flex::row("id").wrap(true).gap(8.0).push(..)`
  默认 `wrap == false` → **走原单行路径，逐像素不变**（hash-gate 守门）。
- **实现形态**：`Flex` 内部两条独立分支（`wrap_measure` / `wrap_arrange`），
  **原 `flex_measure_into` / `flex_arrange_with` 调用逐字未动**。
- **两趟布局**：
  - `measure` 按 `constraints.max.w`（父容器给的**上界**）分行，报总主轴 × 总交叉轴；
  - `arrange` 按 `rect.w`（**实际分配**）**重算分行** —— **以 arrange 为准**（与 CSS 一致）。
  - 两者宽度不同时，`debug_assert!` 会在调试构建报出行数不一致。
- **行内摆放复用**：每一行调用现有 [`flex_arrange_with`]（传 `Edges::ZERO` + 行子切片）——
  因此 **`align` / `justify` 在行内自然生效**，无需重写。
- **行高**：每行 = 该行子项**交叉轴尺寸的最大值**；容器交叉轴 = `Σ行高 + gap × (行数-1)`。
- **已知限制**：
  1. **`arrange` 宽度窄于 `measure` 宽度时，实际行数可能多于预留高度所容纳的行数 → 内容可能溢出预留高度**（与 CSS 同）。
  2. **`Dimension::Flex` 与 `wrap` 同时使用时，flex 因子被忽略**（按内容尺寸分行）——
     因为 flex 宽度取决于同行其他子项，与“先分行再分配”**互相依赖**。
  3. **absolute / aspect-ratio 仍不做**（与“父容器分配 rect”模型冲突）。
