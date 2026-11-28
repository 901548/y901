//! **无障碍语义不变量**（不需要读屏器、不需要 GPU，CI 可跑）。
//!
//! 2026-11-28 做"真机读屏器验收"时，用 `UIAutomationClient` 读 `a11y_demo` 的真实 UIA 树，
//! 发现一串**用户会直接听到**的问题（证据见
//! `docs/evidence/cross-platform/a11y-uia-windows-2026-11-28.log`）：
//!
//! | 现象 | 根因 |
//! |---|---|
//! | 复选框/滑杆被念成"按钮"、告警被念成"分组" | 组件把**主题色槽**（8 个）当成语义角色报出去 |
//! | 名字是 `switch switch-ctl` / `slider slider-ctl` | `semantics().label` 里拼了内部 id |
//! | 按钮的"值"是 `a11y.save` | 命令名被塞进 `value`，读屏器会念出来 |
//! | 整棵列表、告警不在树里 | `VerticalList` 没调 `.exposed(true)`（默认是封禁） |
//! | 输入框只有内容没有名字 | 名字与内容混用同一个字段 |
//!
//! 本文件把这些**逐条钉住**，防止改回去。它们全部是纯 CPU 断言。

use y901_a11y::accesskit::Role as AkRole;
use y901_a11y::to_tree_update_with_map;
use y901_ui::{Flex, Role, Size, TextMetrics, TextStyle, Theme, UiTree};
use y901_widgets::{
    Alert, AlertKind, Button, Checkbox, NumberInput, Slider, Switch, TextInput, TextLabel,
    VerticalList,
};

struct Mono;
impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 与 `a11y_demo` 同构成的一棵树。
fn demo_tree() -> UiTree {
    let mut list = VerticalList::new("a11y-list");
    for i in 1..=3 {
        list = list.push(Box::new(TextLabel::new(
            format!("item-{i}"),
            format!("列表项 {i}"),
        )));
    }
    let root = Flex::column("root")
        .push(Box::new(TextLabel::new("title", "Y901 无障碍验收")))
        .push(Box::new(
            Button::new("save-btn", "保存").on_click("a11y.save", y901_ui::Json::obj()),
        ))
        .push(Box::new(Checkbox::new("check-box", "启用同步", false)))
        .push(Box::new(Switch::new("switch-ctl", true).label("启用同步")))
        .push(Box::new(Slider::new("slider-ctl", 0.5).label("音量")))
        .push(Box::new(
            NumberInput::new("number-ctl", 42.0).label("重试次数"),
        ))
        .push(Box::new(
            TextInput::new("text-ctl").label("备注").with_text("内容"),
        ))
        .push(Box::new(list))
        .push(Box::new(Alert::new(
            "alert",
            AlertKind::Warning,
            "这是一个告警",
        )));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(560.0, 640.0), &Mono);
    tree
}

fn node_by_role(
    update: &y901_a11y::accesskit::TreeUpdate,
    role: AkRole,
) -> Option<&y901_a11y::accesskit::Node> {
    update
        .nodes
        .iter()
        .find(|(_, n)| n.role() == role)
        .map(|(_, n)| n)
}

/// **角色必须报真实语义**：复选框念"复选框"、滑杆念"滑杆"、开关念"开关"、告警念"告警"。
///
/// 旧实现把它们都报成 `button`/`panel`（因为 `Role` 当时被当主题色槽用），
/// 于是读屏器把滑杆念成按钮 —— 用户按方向键改值的那个东西被念成"按钮"。
#[test]
fn semantic_roles_reach_accesskit_instead_of_button_or_panel() {
    let tree = demo_tree();
    let (update, _map) = to_tree_update_with_map(tree.snapshot());
    assert!(
        node_by_role(&update, AkRole::CheckBox).is_some(),
        "复选框必须映射成 AccessKit 的 CheckBox（不是 Button）"
    );
    assert!(
        node_by_role(&update, AkRole::Slider).is_some(),
        "滑杆必须映射成 Slider（不是 Button）"
    );
    assert!(
        node_by_role(&update, AkRole::Switch).is_some(),
        "开关必须映射成 Switch"
    );
    assert!(
        node_by_role(&update, AkRole::Alert).is_some(),
        "告警必须映射成 Alert（不是 GenericContainer）"
    );
}

/// **名字里不得出现内部 id**：`switch switch-ctl` 这种会被读屏器原样念出来。
#[test]
fn no_node_leaks_its_internal_id_into_the_name() {
    let tree = demo_tree();
    let (update, map) = to_tree_update_with_map(tree.snapshot());
    for (id, node) in &update.nodes {
        let Some(y901_id) = map.y901_of(*id) else {
            continue;
        };
        if let Some(label) = node.label() {
            assert!(
                !label.contains(y901_id),
                "节点 `{y901_id}` 的名字里含自己的 id：{label:?}（读屏器会把实现细节念出来）"
            );
        }
    }
}

/// **按钮的 `value` 不得是命令名**（读屏器会念成"保存，a11y.save"）。
#[test]
fn button_does_not_expose_its_command_as_value() {
    let tree = demo_tree();
    let (update, _map) = to_tree_update_with_map(tree.snapshot());
    let button = node_by_role(&update, AkRole::Button).expect("有按钮");
    assert!(
        button.value().is_none_or(|v| !v.contains("a11y.save")),
        "按钮的 value 泄漏了命令名：{:?}",
        button.value()
    );
}

/// **列表与告警必须在无障碍树里**（`VerticalList` 曾因默认 `exposed=false` 整棵消失）。
#[test]
fn list_and_its_items_are_in_the_tree() {
    let tree = demo_tree();
    let (update, _map) = to_tree_update_with_map(tree.snapshot());
    assert!(
        node_by_role(&update, AkRole::List).is_some(),
        "列表必须出现在无障碍树里（否则读屏器看不到项数，也进不去）"
    );
    let items = update
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.role() == AkRole::ListItem || n.label().is_some_and(|l| l.starts_with("列表项"))
        })
        .count();
    assert!(items >= 3, "列表项应逐项可见，实得 {items}");
}

/// **名字与内容分开**：输入框的名字是"备注"，值是"内容"。
#[test]
fn text_input_keeps_name_and_value_separate() {
    let tree = demo_tree();
    let (update, _map) = to_tree_update_with_map(tree.snapshot());
    // 树里有两个 TextInput 类角色（数值输入也是），按名字找我们那个。
    let input = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.role() == AkRole::TextInput && n.label() == Some("备注"))
        .expect("应有名为『备注』的文本框");
    assert_eq!(input.value(), Some("内容"), "值应是内容（名字与内容分开）");
}

/// **语义角色不能改变配色**：`Theme::slot` 把 switch/slider/checkbox 别名回 button 槽、
/// alert 别名回 panel 槽 —— 否则"改无障碍"会顺手改掉所有皮肤（像素门禁立刻红）。
#[test]
fn semantic_roles_keep_the_theme_colours() {
    let theme = Theme::DARK;
    let button = theme.style(Role::BUTTON, y901_ui::State::Normal);
    for (role, name) in [
        (Role::SWITCH, "switch"),
        (Role::SLIDER, "slider"),
        (Role::CHECKBOX, "checkbox"),
    ] {
        assert_eq!(
            theme.style(role, y901_ui::State::Normal),
            button,
            "`{name}` 语义角色必须仍取 button 槽的颜色"
        );
    }
    let panel = theme.style(Role::PANEL, y901_ui::State::Normal);
    assert_eq!(
        theme.style(Role::ALERT, y901_ui::State::Normal),
        panel,
        "`alert` 语义角色必须仍取 panel 槽的颜色"
    );
}
/// **每个组件都能被显式命名**（`.label(..)` 生效），且**默认不编名字**。
///
/// 这是 2026-11-28 无障碍清欠账的验收：13 个组件原本把内部 id 拼进名字
/// （`badge my-badge`、`tree my-tree` …），读屏器会念出来。
/// 现在统一成"有 label 用 label，没有就留空"。
#[test]
fn every_widget_accepts_an_explicit_label_and_defaults_to_empty() {
    use y901_ui::layout::LayoutCache;
    use y901_ui::Rect;
    use y901_widgets::{
        Badge, ComboBox, ContextMenu, EmptyState, ErrorState, Image, MenuBar, Popover, ProgressBar,
        RadioGroup, Skeleton, Spinner, Table, TreeView, VerticalList,
    };

    let mut cache = LayoutCache::new();
    let mut arrange = |w: &mut dyn y901_ui::Widget| {
        w.arrange(Rect::new(0.0, 0.0, 300.0, 200.0), &Mono, &mut cache);
    };

    // (未命名的控件, 命名后的控件, 期望名字)
    let mut named: Vec<(String, String)> = Vec::new();
    macro_rules! probe {
        ($plain:expr, $named:expr, $want:expr) => {{
            let mut a = $plain;
            let mut b = $named;
            arrange(&mut a);
            arrange(&mut b);
            let plain_id = y901_ui::Widget::id(&a).to_string();
            let sem = y901_ui::Widget::semantics(&a);
            assert!(
                !sem.label.contains(&plain_id),
                "{} 未命名时的 label 不得含 id：{:?}",
                plain_id,
                sem.label
            );
            let sem2 = y901_ui::Widget::semantics(&b);
            assert_eq!(sem2.label, $want, "{} 的显式名字应生效", plain_id);
            named.push((plain_id, sem2.label));
        }};
    }

    probe!(
        Badge::new("b", "新"),
        Badge::new("b", "新"),
        "新" // 徽标：文字就是名字
    );
    probe!(
        ComboBox::new("cb", vec!["甲", "乙"]),
        ComboBox::new("cb", vec!["甲", "乙"]).label("城市"),
        "城市"
    );
    probe!(
        ContextMenu::new("cm", vec!["复制".into()]),
        ContextMenu::new("cm", vec!["复制".into()]).label("右键菜单"),
        "右键菜单"
    );
    probe!(
        EmptyState::new("es", "没有数据"),
        EmptyState::new("es", "没有数据"),
        "没有数据" // 标题就是名字
    );
    probe!(
        ErrorState::new("ers", "加载失败"),
        ErrorState::new("ers", "加载失败"),
        "加载失败"
    );
    probe!(
        Image::from_rgba("img", 2, 2, &[0u8; 16]),
        Image::from_rgba("img", 2, 2, &[0u8; 16]).label("公司标志"),
        "公司标志"
    );
    probe!(
        MenuBar::new("mb"),
        MenuBar::new("mb").label("主菜单"),
        "主菜单"
    );
    probe!(
        ProgressBar::new("pb", 0.5),
        ProgressBar::new("pb", 0.5).label("上传进度"),
        "上传进度"
    );
    probe!(
        RadioGroup::new("rg", vec!["小".into(), "大".into()], None),
        RadioGroup::new("rg", vec!["小".into(), "大".into()], None).label("字号"),
        "字号"
    );
    probe!(
        Spinner::new("sp"),
        Spinner::new("sp").label("加载中"),
        "加载中"
    );
    probe!(Table::new("tb"), Table::new("tb").label("订单"), "订单");
    probe!(
        TreeView::new("tv", vec![]),
        TreeView::new("tv", vec![]).label("文件树"),
        "文件树"
    );
    probe!(
        VerticalList::new("vl"),
        VerticalList::new("vl").label("待办"),
        "待办"
    );
    // 骨架屏是装饰：命名了也不该念（这里只要求不念 id）
    let mut sk = Skeleton::new("sk");
    arrange(&mut sk);
    assert!(!y901_ui::Widget::semantics(&sk).label.contains("sk"));
    // 骨架屏与浮层锚点**刻意不提供** `.label()`：它们是纯装饰/定位件，
    // 给它们名字只会让读屏器念一堆没有信息的节点。
    let _ = Popover::new("po", Rect::ZERO, || {
        Box::new(TextLabel::new("po-child", "x"))
    });

    assert!(
        named.len() >= 13,
        "至少要覆盖 13 个组件，实得 {}",
        named.len()
    );
}
/// **实时区域必须带名字 + live 标记** —— 这两条**同时**成立，`accesskit_windows` 才会
/// 发 `UIA_LiveRegionChanged`（见 accesskit_windows `adapter.rs`：
/// `if wrapper.name().is_some() && node.live() != Live::Off`）。
///
/// 为什么值得单独钉住：告警的价值就在"主动打断"；名字为空或 live 丢了，
/// 它就退化成一段普通文字 —— 用户永远不会被告知。
#[test]
fn live_regions_carry_both_a_name_and_a_live_setting() {
    use y901_a11y::accesskit::Live;
    use y901_widgets::{AlertKind, ErrorState, ProgressBar};

    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(Alert::new("a1", AlertKind::Error, "磁盘快满")))
            .push(Box::new(ErrorState::new("e1", "加载失败")))
            .push(Box::new(ProgressBar::new("p1", 0.4).label("上传进度"))),
    ));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let (update, _map) = to_tree_update_with_map(tree.snapshot());

    let live_nodes: Vec<_> = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .filter(|n| n.live() != Some(Live::Off) && n.live().is_some())
        .collect();
    assert!(
        live_nodes.len() >= 3,
        "告警/错误态/进度条都应是 live region，实得 {}",
        live_nodes.len()
    );
    for n in live_nodes {
        assert!(
            n.label().is_some_and(|l| !l.is_empty()),
            "live region 必须有名字，否则 UIA 不会发 LiveRegionChanged（节点 {:?}）",
            n.role()
        );
    }

    // 告警与错误态是断言式（打断），进度是礼貌式（排队）
    let alert = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.label() == Some("磁盘快满"))
        .expect("告警节点");
    assert_eq!(alert.live(), Some(Live::Assertive), "告警必须是 assertive");
    let progress = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.label() == Some("上传进度"))
        .expect("进度节点");
    assert_eq!(progress.live(), Some(Live::Polite), "进度应是 polite");
}
