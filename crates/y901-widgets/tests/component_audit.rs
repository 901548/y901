//! 组件库审计:跨组件契约 + 行为探针(只加测试,不改组件源码)。
//!
//! 三类用例:
//! 1. **契约锁定**——当前正确的行为,防止回归;
//! 2. **行为探针**——对 review 疑点的验证,通过 = 已修,失败 = 坐实;
//! 3. **已知问题工单**——`#[ignore = "已知问题:…"]`,修复后去掉 ignore 应转绿。
//!
//! 为什么不用组件内嵌 `#[cfg(test)]`:这里是**跨组件**的横向契约,
//! 与具体组件的单元测试分层不同(单测管自己,审计管全局)。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Key, Pos, Rect, RenderCtx, Size, TextMetrics,
    TextStyle, UiEvent, Widget,
};
use y901_widgets::{
    Alert, AlertKind, Badge, Button, ComboBox, EmptyState, ErrorState, Popover, Spinner, Switch,
    Table, TableColumn, TextInput, TextLabel,
};

// ---------- 测试替身 ----------

/// 固定度量:每字符 7px 宽、行高 14、scale=1(与各组件单测同一约定)。
struct M;
impl TextMetrics for M {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 7.0, 14.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        14.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 记录型渲染上下文:统计绘制调用、记录文本、可注入动画进度。
///
/// 只实现 4 个无默认值的方法(size/fill_rect/stroke_rect/draw_text)
/// + 覆写 `animation`(计数,验证"render 消费动画")。其余全走默认实现。
struct Rec {
    anim_value: f32,
    /// `ctx.animation(..)` 被调用的次数(内部可变性:签名是 `&self`)。
    anim_calls: Cell<u32>,
    fills: u32,
    texts: Vec<String>,
}

impl Rec {
    fn new(anim_value: f32) -> Self {
        Self {
            anim_value,
            anim_calls: Cell::new(0),
            fills: 0,
            texts: Vec::new(),
        }
    }
}

impl TextMetrics for Rec {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 7.0, 14.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        14.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _rect: Rect, _color: Color) {
        self.fills += 1;
    }
    fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {
        // 计入 fills,便于"至少画了东西"类断言
        self.fills += 1;
    }
    fn draw_text(&mut self, _origin: Pos, text: &str, _style: &TextStyle) -> f32 {
        self.texts.push(text.to_string());
        text.chars().count() as f32 * 7.0
    }
    fn push_clip(&mut self, _rect: Rect) {
        self.fills += 1;
    }
    fn pop_clip(&mut self) {}
    fn animation(&self, _key: u64) -> f32 {
        self.anim_calls.set(self.anim_calls.get() + 1);
        self.anim_value
    }
}

/// 组件集合:今天新写的 10 个 + 3 个老组件代表。
/// 构造参数故意用最小形态(不带 builder),锁定默认行为。
macro_rules! for_each_probe_component {
    ($name:ident) => {{
        let $name: Vec<Box<dyn Widget>> = vec![
            Box::new(Alert::new("p-alert", AlertKind::Info, "标题")),
            Box::new(Alert::new("p-alert2", AlertKind::Error, "标题").description("说明")),
            Box::new(Switch::new("p-switch", false)),
            Box::new(Spinner::new("p-spinner")),
            Box::new(Table::new("p-table")),
            Box::new(EmptyState::new("p-empty", "暂无数据")),
            Box::new(ErrorState::new("p-error", "加载失败")),
            Box::new(Badge::new("p-badge", "beta")),
            Box::new(ComboBox::new("p-combo", vec!["甲", "乙"])),
            Box::new(Popover::new(
                "p-popover",
                Rect::new(0.0, 0.0, 80.0, 24.0),
                || Box::new(TextLabel::new("p-pop-inner", "内容")) as Box<dyn Widget>,
            )),
            Box::new(Button::new("p-button", "确定")),
            Box::new(TextInput::new("p-input")),
            Box::new(TextLabel::new("p-label", "文字")),
        ];
        $name
    }};
}

fn arrange_each(components: &mut [Box<dyn Widget>], cache: &mut LayoutCache) {
    for (i, c) in components.iter_mut().enumerate() {
        c.arrange(
            Rect::new(0.0, 0.0 + i as f32 * 100.0, 200.0, 50.0),
            &M,
            cache,
        );
    }
}

// ---------- 一、语义面契约 ----------

/// 已知问题(审计发现):TextInput 的语义 label 为空——外部工具只能靠 id 辨认节点。
/// 其余组件都有可读 label;修复后去掉 ignore 应转绿。
#[test]
fn text_input_semantics_has_label() {
    let input = TextInput::new("p-input");
    let sem = input.semantics();
    assert!(
        !sem.label.is_empty(),
        "TextInput 应提供语义 label(当前为空,只有 id 可用)"
    );
}

/// 所有对外组件必须 `exposed(true)`:语义面默认封禁,
/// 忘了 `.exposed(true)` 的组件对外部工具(AI/RPC)是不可见的。
/// label 非空要求覆盖全部组件,TextInput 暂列豁免(见上方工单)。
#[test]
fn semantics_exposed_contract() {
    const LABEL_EXEMPT: &[&str] = &["p-input"];
    let mut comps = for_each_probe_component!(comps);
    let mut cache = LayoutCache::new();
    arrange_each(&mut comps, &mut cache);
    for c in &comps {
        let sem = c.semantics();
        assert!(
            sem.exposed_to_ai,
            "{} 的 semantics 未暴露(exposed_to_ai=false)——外部工具看不到它",
            c.id()
        );
        if !LABEL_EXEMPT.contains(&c.id()) {
            assert!(
                !sem.label.is_empty(),
                "{} 的语义 label 为空——外部工具无法辨认节点",
                c.id()
            );
        }
    }
}

/// 有状态组件的 value 必须反映当前值(Switch 的 on/off、ComboBox 的显示文本、Alert 的标题)。
#[test]
fn value_semantics_reflects_state() {
    let sw = Switch::new("s", false);
    let sem = sw.semantics();
    assert_eq!(sem.value, "off", "未选中 Switch 的 value 应为 off");
    assert!(
        sem.props
            .iter()
            .any(|(k, v)| k == "checked" && v == "false"),
        "应带 checked=false 属性"
    );

    let sw2 = Switch::new("s2", true);
    let sem2 = sw2.semantics();
    assert_eq!(sem2.value, "on", "选中 Switch 的 value 应为 on");

    let cb = ComboBox::new("cb", vec!["甲", "乙"]);
    let sem3 = cb.semantics();
    assert_eq!(sem3.value, "请选择", "未选中 ComboBox 应显示占位文本");

    let cb2 = ComboBox::new("cb2", vec!["甲", "乙"]).selected(Some(1));
    let sem4 = cb2.semantics();
    assert_eq!(sem4.value, "乙", "选中后 value 应为所选项文本");

    let alert = Alert::new("a", AlertKind::Warning, "磁盘快满");
    let sem5 = alert.semantics();
    assert_eq!(sem5.value, "磁盘快满", "Alert 的 value 应为标题");
    assert!(
        sem5.props
            .iter()
            .any(|(k, v)| k == "kind" && v == "warning"),
        "Alert 应带 kind=warning 属性"
    );
}

// ---------- 二、布局契约 ----------

/// measure 输出必须有限且落在约束内(无限值会在光栅化里"不可见但不报错")。
#[test]
fn measure_respects_loose_constraints() {
    let mut comps = for_each_probe_component!(comps);
    let mut cache = LayoutCache::new();
    for c in comps.iter_mut() {
        let s = c.measure(&M, Constraints::loose(Size::new(200.0, 100.0)), &mut cache);
        assert!(
            s.w.is_finite() && s.h.is_finite(),
            "{} measure 返回了非有限尺寸 {s:?}",
            c.id()
        );
        assert!(
            s.w <= 200.0 && s.h <= 100.0,
            "{} measure 违反约束: {s:?} > (200,100)",
            c.id()
        );
    }
}

/// 无界约束(ScrollView 内容区)下 measure 也必须有限——
/// `INFINITY` 只表示"你可以任意大",绝不允许出现在返回值里。
#[test]
fn measure_under_unbounded_constraints_stays_finite() {
    let mut comps = for_each_probe_component!(comps);
    let mut cache = LayoutCache::new();
    for c in comps.iter_mut() {
        let s = c.measure(
            &M,
            Constraints::loose(Size::new(f32::INFINITY, f32::INFINITY)),
            &mut cache,
        );
        assert!(
            s.w.is_finite() && s.h.is_finite(),
            "{} 在无界约束下返回了非有限尺寸 {s:?}",
            c.id()
        );
    }
}

// ---------- 三、渲染防御 ----------

/// 已知问题(审计发现):Button / TextLabel 无零矩形防御——未布局就绘制。
/// 框架的 needs_paint 保证未布局节点实际不会被渲染,所以只是防线缺口,不是画面 bug;
/// 但与其他组件的"判空早退"纪律不一致。修复后去掉 ignore 应转绿。
#[test]
fn legacy_components_render_without_layout_draw_nothing() {
    let mut b = Button::new("p-button", "确定");
    let mut rec = Rec::new(1.0);
    b.render(&mut rec);
    assert_eq!(rec.fills, 0, "未布局的 Button 不应产生绘制调用");

    let mut l = TextLabel::new("p-label", "文字");
    let mut rec2 = Rec::new(1.0);
    l.render(&mut rec2);
    assert!(
        rec2.texts.is_empty(),
        "未布局的 TextLabel 不应画文字(画了 {:?})",
        rec2.texts
    );
}

/// 未布局(rect 为零)时 render 必须安全:要么早退不画,要么不 panic。
/// 框架的 `needs_paint` 保证未布局节点不会被画,这里是组件侧的最后防线。
/// Button/TextLabel 暂列豁免(见上方工单)。
#[test]
fn render_without_layout_does_not_panic_and_draws_nothing() {
    const ZERO_RECT_EXEMPT: &[&str] = &["p-button", "p-label"];
    let mut comps = for_each_probe_component!(comps);
    for c in comps.iter_mut() {
        let mut rec = Rec::new(1.0);
        c.render(&mut rec);
        if ZERO_RECT_EXEMPT.contains(&c.id()) {
            continue;
        }
        assert_eq!(
            rec.fills,
            0,
            "{} 未布局就画了 {} 次——应先判空矩形早退",
            c.id(),
            rec.fills
        );
        assert!(
            rec.texts.is_empty(),
            "{} 未布局就画了文字 {:?}",
            c.id(),
            rec.texts
        );
    }
}

/// Switch 渲染必须消费框架动画(`ctx.animation(key)`)——
/// 这是"滑块平滑过渡"的数据来源;不读动画 = 又回到瞬间跳变。
#[test]
fn switch_render_consumes_animation() {
    let mut sw = Switch::new("sw", true);
    let mut cache = LayoutCache::new();
    sw.arrange(Rect::new(0.0, 0.0, 44.0, 24.0), &M, &mut cache);
    let mut rec = Rec::new(0.5);
    sw.render(&mut rec);
    assert!(
        rec.anim_calls.get() >= 1,
        "Switch::render 未查询动画进度(ctx.animation)——滑块会瞬间跳变"
    );
}

// ---------- 四、行为探针 ----------

/// Switch:Enter 与空格都能切换;配置了 command 时切换应发出 Command
/// (组件发命令、应用层解释;未配置则静默——emit 是条件发送)。
#[test]
fn switch_enter_and_space_toggle_and_emit_command() {
    let mut sw = Switch::new("sw", false).command(y901_ui::Command::app(
        "audit.switch.toggled",
        y901_ui::json::Json::obj(),
    ));
    let mut cache = LayoutCache::new();
    sw.arrange(Rect::new(0.0, 0.0, 44.0, 24.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));

    let r = sw.event(
        &mut ctx,
        &UiEvent::Key {
            key: Key::Enter,
            modifiers: Default::default(),
        },
    );
    assert_eq!(r, EventResult::Redraw);
    assert!(sw.checked(), "Enter 应切换开关");
    assert_eq!(
        ctx.commands.len(),
        1,
        "配置了 command 的切换应发出一条 Command(由应用层解释)"
    );

    let mut ctx2 = EventCtx::new(None, Size::new(200.0, 100.0));
    sw.event(
        &mut ctx2,
        &UiEvent::Key {
            key: Key::Char(' '),
            modifiers: Default::default(),
        },
    );
    assert!(!sw.checked(), "空格应切换开关(Key::Char(' ') 即空格)");

    // 未配置 command 的开关:切换不产生命令(条件发送),但状态照变
    let mut bare = Switch::new("bare", false);
    let mut ctx3 = EventCtx::new(None, Size::new(200.0, 100.0));
    bare.event(
        &mut ctx3,
        &UiEvent::Key {
            key: Key::Enter,
            modifiers: Default::default(),
        },
    );
    assert!(bare.checked());
    assert_eq!(ctx3.commands.len(), 0, "未配置 command 时不应发命令");
}

/// 已知问题(审计发现):**键盘切换(Enter/空格)不发起滑块动画**——
/// 只有 MouseUp 路径调用了 `start_animation`,键盘路径直接跳变,
/// 与鼠标路径的平滑过渡不一致。修复后去掉 ignore 应转绿。
#[test]
fn switch_keyboard_toggle_starts_animation() {
    let mut sw = Switch::new("sw", false);
    let mut cache = LayoutCache::new();
    sw.arrange(Rect::new(0.0, 0.0, 44.0, 24.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
    sw.event(
        &mut ctx,
        &UiEvent::Key {
            key: Key::Enter,
            modifiers: Default::default(),
        },
    );
    assert_eq!(
        ctx.anim_requests.len(),
        1,
        "键盘切换应与鼠标切换一样发起滑块动画"
    );
}

/// 鼠标切换发起动画(对照:键盘路径见上方工单)。
#[test]
fn switch_mouse_toggle_starts_animation() {
    let mut sw = Switch::new("sw", false);
    let mut cache = LayoutCache::new();
    sw.arrange(Rect::new(0.0, 0.0, 44.0, 24.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(200.0, 100.0));
    sw.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(20.0, 12.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert_eq!(ctx.anim_requests.len(), 1, "鼠标切换应发起滑块动画");
}

/// Spinner 相位 8 tick 一循环,且每次 tick 都要求重绘。
#[test]
fn spinner_phase_wraps_every_8_ticks() {
    let mut sp = Spinner::new("sp");
    let mut cache = LayoutCache::new();
    sp.arrange(Rect::new(0.0, 0.0, 24.0, 24.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(100.0, 100.0));
    for _ in 0..8 {
        assert_eq!(sp.event(&mut ctx, &UiEvent::Tick), EventResult::Redraw);
    }
    assert_eq!(sp.phase(), 0, "8 个相位后应回到起点");
}

/// Alert 的关闭按钮命中区应跟随**布局后**矩形(非原点);
/// 关闭走 Command(约定:组件发命令、应用层解释),不发闭包回调。
#[test]
fn alert_dismiss_hit_region_and_command() {
    let mut a = Alert::new("a", AlertKind::Error, "出错了").on_dismiss(y901_ui::Command::app(
        "audit.alert.dismiss",
        y901_ui::json::Json::obj(),
    ));
    let mut cache = LayoutCache::new();
    a.arrange(Rect::new(100.0, 50.0, 400.0, 48.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(600.0, 200.0));
    // 关闭按钮在右端 32px 区:命中其中心
    let r = a.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(484.0, 74.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert_eq!(r, EventResult::Redraw, "关闭按钮中心应命中");
    assert_eq!(
        ctx.commands.len(),
        1,
        "关闭应产出一条 Command(组件只搬运语义,由应用层解释)"
    );
    // 条幅中部:不应命中、不应有命令
    let r2 = a.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(300.0, 74.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert_eq!(r2, EventResult::Ignored, "条幅中部不应触发关闭");
}

/// Popover 的命中必须用**布局后** rect,不能用构造期 anchor:
/// anchor 写 (10,10),组件被布局摆到 (200,50) 时,点 (210,60) 应弹出、点 (15,15) 不应弹出。
#[test]
fn popover_hits_arranged_rect_not_constructor_anchor() {
    let mut p = Popover::new("p", Rect::new(10.0, 10.0, 80.0, 24.0), || {
        Box::new(TextLabel::new("inner", "x")) as Box<dyn Widget>
    });
    let mut cache = LayoutCache::new();
    p.arrange(Rect::new(200.0, 50.0, 80.0, 24.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(400.0, 300.0));

    p.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(15.0, 15.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert!(
        ctx.popup_requests.is_empty(),
        "构造期 anchor 的位置(15,15)不应命中——组件实际在 (200,50)"
    );
    p.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(210.0, 60.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert_eq!(ctx.popup_requests.len(), 1, "布局后区域内点击应弹出");
    assert!(
        (ctx.popup_requests[0].anchor.x - 200.0).abs() < 0.01,
        "弹出锚点应为布局后位置,而不是构造期 anchor"
    );
}

/// Popover 每次打开都重建内容(框架 popup 不与宿主共享状态——已知取舍)。
/// 该测试**锁定**这个行为,防止有人误以为状态会保留。
#[test]
fn popover_rebuilds_content_on_every_open() {
    let opens = Rc::new(Cell::new(0u32));
    let o2 = opens.clone();
    let mut p = Popover::new("p", Rect::new(0.0, 0.0, 80.0, 24.0), move || {
        o2.set(o2.get() + 1);
        Box::new(TextLabel::new("inner", "x")) as Box<dyn Widget>
    });
    let mut cache = LayoutCache::new();
    p.arrange(Rect::new(0.0, 0.0, 80.0, 24.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(200.0, 200.0));
    p.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(40.0, 12.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    p.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(40.0, 12.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert_eq!(opens.get(), 2, "每次打开都应重新构造内容工厂");
}

/// Table 的 selected() 越界必须被夹掉(不能让渲染阶段访问不存在的行)。
#[test]
fn table_selected_out_of_range_is_rejected() {
    let t = Table::new("t")
        .column(TableColumn::flex("名称"))
        .row(vec!["a".into()])
        .row(vec!["b".into()])
        .selected(Some(5));
    assert_eq!(t.selected_index(), None, "越界选中应被拒绝");
    let t2 = Table::new("t2").selected(Some(0));
    assert_eq!(t2.selected_index(), None, "空表不允许任何选中");
}

// ---------- 五、已知问题工单(修复后去掉 ignore 应转绿) ----------

/// 已知问题:ComboBox 无键盘导航。
/// 期望:聚焦后 ↑↓ 在选项间移动、Enter 确认、Esc 关闭下拉
/// (Menu 已有完整的键盘导航实现可对照搬运)。
#[test]
fn combo_box_keyboard_navigation() {
    let mut cb = ComboBox::new("cb", vec!["甲", "乙", "丙"]).selected(Some(0));
    let mut cache = LayoutCache::new();
    cb.arrange(Rect::new(0.0, 0.0, 200.0, 32.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
    cb.event(
        &mut ctx,
        &UiEvent::Key {
            key: Key::Down,
            modifiers: Default::default(),
        },
    );
    assert_eq!(
        cb.selected_index(),
        Some(1),
        "↓ 应把选中移到下一项(当前:完全无响应)"
    );
}

/// 已知问题:Table 无键盘导航。
/// 期望:↑↓ 在行间移动选中(TreeView 的键盘模式可搬运)。
#[test]
fn table_keyboard_navigation() {
    let mut t = Table::new("t")
        .column(TableColumn::flex("名称"))
        .row(vec!["a".into()])
        .row(vec!["b".into()]);
    let mut cache = LayoutCache::new();
    t.arrange(Rect::new(0.0, 0.0, 200.0, 100.0), &M, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
    t.event(
        &mut ctx,
        &UiEvent::Key {
            key: Key::Down,
            modifiers: Default::default(),
        },
    );
    assert_eq!(
        t.selected_index(),
        Some(0),
        "↓ 应选中第一行(当前:完全无响应)"
    );
}

// ---------- 六、源码级自动审计 ----------

/// **自动检测**:任何消费 `UiEvent::Tick` 的组件必须声明 `wants_tick`。
///
/// 与 `tick_consumers.rs` 的手工名单互补——那份名单可能**漏登记**新组件
/// (Spinner 曾因此静默失灵);这份扫描以"实际用了 Tick"为准,自动抓漏网之鱼。
#[test]
fn every_tick_user_declares_wants_tick() {
    use std::fs;
    use std::path::Path;

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = Vec::new();
    let entries = fs::read_dir(&src).expect("读取 src 目录");
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        if text.contains("UiEvent::Tick") && !text.contains("fn wants_tick(&self)") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            offenders.push(name.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "以下组件消费了 UiEvent::Tick 却没实现 wants_tick——\
         tree.rs 的广播门控不会给它们发 Tick,运行时静默不动:{offenders:?}"
    );
}
