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