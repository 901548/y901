# Y901 框架使用手册

> 面向第一次接触 Y901 的开发者。读完你应该能用 `y901-ui` 从零搭出一个可交互界面，
> 并且明白**为什么框架里没有一行业务代码**。

---

## 1. 三层心智模型

```
┌──────────────────────────────────────────────┐
│ y901-app      应用：组装组件、解释业务命令      │  ← 只有这里知道"清屏""切换页面"
├──────────────────────────────────────────────┤
│ y901-widgets  组件：Button / Terminal / 你的   │  ← 互不认识，只发命令
├──────────────────────────────────────────────┤
│ y901-ui       框架：布局 / 事件 / 语义树 / RPC  │  ← 零依赖，零业务
└──────────────────────────────────────────────┘
```

三条铁律：

1. **框架不认识平台**：`RenderCtx` 管画，`TextMetrics` 管量。
   唤醒由应用层负责 —— 后台线程 `PostMessage` 打断消息循环，组件用 `UiEvent::Tick` 轮询；
   框架本身**没有** `Waker` 抽象。
2. **框架不认识业务**：它不知道什么是"终端""文件树""清屏"。
3. **组件互不认识**：组件之间只通过 `Command` 说话，语义由应用解释。

---

## 2. 第一个组件：只显示文字

```rust
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, FlexItem, Pos, Rect, RenderCtx, Role, Semantics,
    Size, TextStyle, TextMetrics, UiEvent, Widget,
};

pub struct TextLabel {
    id: String,
    text: String,
    rect: Rect,
    item: FlexItem,
}

impl TextLabel {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { id: id.into(), text: text.into(), rect: Rect::ZERO, item: FlexItem::default() }
    }
    pub fn flex_item(mut self, item: FlexItem) -> Self { self.item = item; self }
}

impl Widget for TextLabel {
    fn id(&self) -> &str { &self.id }

    fn role(&self) -> Role { Role::TEXT }

    fn flex(&self) -> FlexItem { self.item }

    // ① 报尺寸：父容器据此分配空间
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints) -> Size {
        let style = TextStyle::fg(Color::WHITE);
        c.constrain(Size::new(
            m.measure_text(&self.text, &style).w,
            m.line_height(&style),
        ))
    }

    // ② 收矩形：父容器算好绝对坐标后回传
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics) { self.rect = rect; }

    fn rect(&self) -> Rect { self.rect }

    // ③ 收事件
    fn event(&mut self, _ctx: &mut EventCtx, ev: &UiEvent) -> EventResult {
        if let UiEvent::Action { name, args } = ev {
            if name == "set_text" {
                if let Some(t) = args.get("text").and_then(|v| v.as_str()) {
                    self.text = t.to_string();
                    return EventResult::Relayout;   // 文字变了，尺寸可能变
                }
            }
        }
        EventResult::Ignored
    }

    // ④ 出图：只能通过 RenderCtx
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.draw_text(
            Pos::new(self.rect.x, self.rect.y),
            &self.text,
            &TextStyle::fg(Color::WHITE),
        );
    }

    // ⑤ 语义：Agent 看到的东西
    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(self.text.clone())
            .action("set_text")
    }
}
```

**没有任何地方引用 D3D11、Windows 或别的组件。** 这就是一个合法组件。

---

## 3. 第一个容器：纵向列表

容器的差别只在于：`arrange` 时要**继续给子节点排布**。

```rust
use y901_ui::layout::{flex_arrange, flex_measure};
use y901_ui::{Axis, Edges, ...};

pub struct VerticalList {
    id: String,
    children: Vec<Box<dyn Widget>>,
    gap: f32,
    rect: Rect,
}

impl Widget for VerticalList {
    fn id(&self) -> &str { &self.id }
    fn role(&self) -> Role { Role::LIST }

    // 让框架能遍历/路由
    fn children(&self) -> &[Box<dyn Widget>] { &self.children }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }

    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints) -> Size {
        flex_measure(Axis::Vertical, Edges::ZERO, self.gap, &mut self.children, m, c)
    }

    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics) {
        self.rect = rect;
        // 框架提供 flex 算法，但"要不要用"是容器的自由
        flex_arrange(Axis::Vertical, Edges::ZERO, self.gap, &mut self.children, rect, m);
    }

    fn rect(&self) -> Rect { self.rect }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn event(&mut self, _ctx: &mut EventCtx, _ev: &UiEvent) -> EventResult {
        EventResult::Ignored      // 容器不管事，事件由框架投给命中的子节点
    }
}
```

（完整实现见 `crates/y901-widgets/src/list.rs`，也可以直接用 `Flex::column`。）

---

## 4. 组装一棵树

```rust
use y901_ui::{Flex, FlexItem, UiTree};

let root = Flex::column("root")
    .padding(Edges::all(16.0))
    .gap(12.0)
    .push(Box::new(TextLabel::new("title", "Hello Y901").flex_item(FlexItem::height(24.0))))
    .push(Box::new(
        VerticalList::new("list")
            .gap(4.0)
            .flex_item(FlexItem::fill())
            .push(Box::new(TextLabel::new("a", "row 1").flex_item(FlexItem::height(20.0))))
            .push(Box::new(TextLabel::new("b", "row 2").flex_item(FlexItem::height(20.0)))),
    ));

let mut tree = UiTree::new(Box::new(root));
tree.layout(Size::new(800.0, 600.0), &metrics);   // metrics 由后端提供
```

`Flex::row` / `Flex::column` 就是框架自带的容器；`FlexItem::height(24.0)` 表示固定高度，
`FlexItem::fill()` 表示吃掉剩余空间。

---

## 5. 命令：组件不认识的"意图"

假设按钮要清空终端。**按钮不知道终端存在**：

```rust
// 在 y901-app 里组装
Button::new("btn_clear", "Clear")
    .on_click("terminal.clear", Json::obj().set("target", "main_terminal"))
```

按钮内部只做一件事：

```rust
ctx.emit_app(self.command.clone(), self.args.clone());   // 交给应用层
```

应用层（唯一知道语义的地方）：

```rust
fn process_commands(app: &mut App) {
    for cmd in app.tree.drain_commands() {
        let Command::App { name, args } = cmd else { continue };
        match name.as_str() {
            "terminal.clear" => {
                let target = args.get("target").and_then(|v| v.as_str()).unwrap();
                app.tree.dispatch_to(target, &UiEvent::Action {
                    name: "clear".into(),
                    args: Json::obj(),
                });
            }
            "demo.switch" => switch_demo(app, args.get("to").and_then(|v| v.as_f64()).unwrap() as u8),
            other => println!("unhandled: {other}"),
        }
    }
}
```

两种命令：

| 变体 | 谁来处理 | 例子 |
|---|---|---|
| `Command::Node { target, action, args }` | **框架自己**路由成 `UiEvent::Action` | 一个容器让子项滚动到可见 |
| `Command::App { name, args }` | **应用层**解释 | `terminal.clear`、`demo.switch` |

框架 `drain_commands()` 只做搬运：Node 命令自己投递，App 命令原样交回。
它**永远不会**解析 `name`。

---

## 6. 用 RPC 驱动同一个界面

启动 RPC 后（`y901_ui::rpc::spawn("127.0.0.1:9410", tx)`），Agent 侧：

```jsonc
// 看整棵树（含每个节点矩形、角色、语义、可用动作）
{"id":1,"method":"get_tree"}

// 向某个节点投递事件
{"id":2,"method":"dispatch","params":{
   "id":"main_terminal",
   "event":{"type":"text","text":"ls\r"}}}

// 调用某个节点的动作
{"id":3,"method":"invoke","params":{
   "id":"main_terminal","action":"clear","args":{}}}

// 切换焦点
{"id":4,"method":"focus","params":{"id":"btn_clear"}}

// 健康检查
{"id":5,"method":"ping"}
```

`get_tree` 返回示例（截断）：

```json
{"id":"root","role":"panel","rect":{"x":0,"y":0,"w":1182,"h":713},
 "children":[
   {"id":"titlebar","role":"panel","rect":{...},"children":[
     {"id":"btn_clear","role":"button","rect":{"x":10,"y":6,"w":84,"h":22},
      "label":"Clear","value":"terminal.clear","actions":["click"]}]},
   {"id":"main_terminal","role":"terminal","rect":{"x":0,"y":34,"w":1182,"h":679},
    "actions":["inject_text","clear","focus"],
    "props":{"cols":"147","rows":"42","cursor_col":"12"}}]}
```

注意 `role` 是**字符串**：框架只内置 `panel/button/text/list/...`，
`"terminal"` 是组件库自己声明的（`y901_widgets::ROLE_TERMINAL`）。

---

## 7. 测试：不需要窗口，也不需要 GPU

`RenderCtx` 是 trait，所以可以直接写一个记录器：

```rust
#[derive(Default)]
struct Recorder { rects: Vec<(Rect, Color)>, texts: Vec<(Pos, String)> }

impl TextMetrics for Recorder {
    fn measure_text(&self, t: &str, _: &TextStyle) -> Size { Size::new(t.len() as f32 * 8.0, 16.0) }
    fn line_height(&self, _: &TextStyle) -> f32 { 16.0 }
    fn scale(&self) -> f32 { 1.0 }
}

impl RenderCtx for Recorder {
    fn size(&self) -> Size { Size::new(800.0, 600.0) }
    fn fill_rect(&mut self, r: Rect, c: Color) { self.rects.push((r, c)); }
    fn stroke_rect(&mut self, r: Rect, c: Color, _w: f32) { self.rects.push((r, c)); }
    fn draw_text(&mut self, p: Pos, t: &str, _: &TextStyle) -> f32 {
        self.texts.push((p, t.to_string())); 0.0
    }
    fn push_clip(&mut self, _: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
fn label_renders() {
    let mut tree = UiTree::new(Box::new(TextLabel::new("t", "hello")));
    tree.layout(Size::new(200.0, 50.0), &Recorder::default());
    let mut rec = Recorder::default();
    tree.render(&mut rec);
    assert_eq!(rec.texts[0].1, "hello");
}
```

完整示例见 `crates/y901-ui/tests/framework.rs`（布局、命中、命令路由、RPC 往返）。

---

## 8. 写组件时的检查清单

- [ ] 只依赖 `y901_ui`，不 `use windows_sys` / `wgpu` / `d3d11`
- [ ] 不在组件里出现别的组件的 id 或类型
- [ ] 需要联动时发 `Command`，不要直接调用别人
- [ ] `measure` 必须返回内容真实尺寸（父容器依赖它）
- [ ] `arrange` 里缓存 rect；容器还要继续排布子节点
- [ ] `event` 返回 `Ignored` 表示不关心（会冒泡），`Handled` 表示消费掉
- [ ] `semantics()` 暴露 label/actions/state，Agent 才能操作你
- [ ] 状态是组件自己的字段；不要塞进框架

---

## 9. 两个 Demo

`cargo run -p y901-app` 启动后：

| Demo | 内容 | 验证点 |
|---|---|---|
| 1 | 标题栏两个按钮 + 终端 | 点 `Clear` → 应用拦截 `terminal.clear` → 终端收到 `clear` 动作清屏 |
| 2 | 标签 / 按钮 / VerticalList | 点 `Say Hi` → 应用拦截 `label.set` → 状态标签文字变化 |

两个 Demo 之间用 `demo.switch` 命令切换，全程没有任何组件互相引用。

---

## 10. 常见误解

**Q：为什么不直接在 Button 里调用 `terminal.clear()`？**
A：那样 Button 就必须 `use TerminalWidget`，组件库立刻退化成一张网；而且任何新页面想复用 Button，
都得带上终端。发命令则只需要一个字符串。

**Q：为什么 `Terminal` 的 role 不在框架枚举里？**
A：框架枚举里每多一个业务名词，就意味着"新增业务要改框架"。`Role(&'static str)` 让组件库
自己定义 `ROLE_TERMINAL`，框架永远不用改。

**Q：命令会不会变成万能 RPC，什么业务都往里塞？**
A：约定是——`Command::App` 只允许出现在**交互组件**（Button/Menu/快捷键）；
容器禁止发命令。命令名用 `域.动作` 格式（`terminal.clear`），应用层集中 switch，
一眼能看出业务边界。
