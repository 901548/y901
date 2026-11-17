//! **IBus 客户端**（纯 Rust：`zbus` + `zvariant`，无 C 依赖）。
//!
//! ## 为什么连"IBus 自己的总线"而不是会话总线
//!
//! 实测（WSL2 Ubuntu 22.04，IBus 1.5.26）：`org.freedesktop.IBus` 这个名字**不在会话总线**上，
//! 而是在 IBus 私有的 D-Bus 服务器里。地址写在
//! `~/.config/ibus/bus/<machine-id>-<display>` 的 `IBUS_ADDRESS=` 一行（或 `IBUS_ADDRESS` 环境变量），
//! 形如 `unix:abstract=/home/xxx/.cache/ibus/dbus-xxxx`。
//! `zbus` 的 `blocking::connection::Builder::ibus()` 正是为此设计：它会调用 `ibus address` 拿到地址。
//!
//! 另有一个**必须遵守的约束**：`CreateInputContext` 创建的上下文**绑定创建者的 D-Bus 连接**，
//! 连接一断，上下文立即失效（实测：每次 `gdbus call` 都是新连接，第二次调用就报
//! "Object does not exist at path"）。所以本实现把连接**持有到 `IbusIme` 生命周期结束**。
//!
//! ## 信号载荷是 IBus 的 GVariant 对象
//!
//! `CommitText` / `UpdatePreeditText` 的文本参数是 `v`（variant），里面是 IBus 序列化对象：
//! `(class_name, properties, text, attrs)` —— 例如
//! `('IBusText', {}, '你好', ('IBusAttrList', {}, []))`。
//! 我们只关心其中的 `text`：用递归扫描取**第一个非空 `Value::Str`**，对当前需求足够稳。

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type as MessageType;
use zbus::MatchRule;
use zbus::zvariant::{OwnedObjectPath, Value};

use super::{drain, ImeClient, ImeEvent, KeyOutcome};

/// IBus 输入上下文接口名。
const IFACE: &str = "org.freedesktop.IBus.InputContext";
/// IBus 守护进程的服务名与对象路径。
const SERVICE: &str = "org.freedesktop.IBus";
const ROOT: &str = "/org/freedesktop/IBus";

/// IBus 能力位：`PREEDIT_TEXT = 1<<0`、`FOCUS = 1<<1`。
///
/// 必须在上 `FocusIn` 之前设置，否则 IBus 会回
/// `The input context does not support focus.`（实测）。
const CAP_PREEDIT: u32 = 1;
const CAP_FOCUS: u32 = 2;

/// IBus 输入法客户端。
pub struct IbusIme {
    /// D-Bus 连接：**必须持有**，否则输入上下文会随连接销毁（见模块文档）。
    conn: Arc<Connection>,
    /// 上下文对象路径（打印日志与匹配信号用）。
    ctx_path: OwnedObjectPath,
    /// 信号线程 → 主循环的事件通道。
    rx: Receiver<ImeEvent>,
    /// 信号线程句柄（`Drop` 时不 join，只让通道关闭；线程随连接断开自然退出）。
    _thread: JoinHandle<()>,
}

impl IbusIme {
    /// 连接 IBus 并创建输入上下文。
    ///
    /// 输入：`engine` = 期望的输入法引擎名（例如 `"libpinyin"`；空串表示用 IBus 当前默认引擎）。
    /// 返回：`Err` = IBus 不可用（未运行 / 没有 `ibus` 命令 / D-Bus 出错），调用方应降级为 `NoIme`。
    pub fn connect(engine: &str) -> Result<Self, String> {
        // `Builder::ibus()` 会执行 `ibus address` 取地址；失败说明 IBus 没在跑。
        let conn = zbus::blocking::connection::Builder::ibus()
            .map_err(|e| format!("IBus 地址获取失败：{e}"))?
            .build()
            .map_err(|e| format!("IBus 连接失败：{e}"))?;
        let conn = Arc::new(conn);

        // 创建输入上下文（上下文绑定本连接）。
        let root = Proxy::new(conn.as_ref(), SERVICE, ROOT, "org.freedesktop.IBus")
            .map_err(|e| format!("IBus 根对象代理失败：{e}"))?;
        let ctx_path: OwnedObjectPath = root
            .call("CreateInputContext", &("y901",))
            .map_err(|e| format!("CreateInputContext 失败：{e}"))?;

        // 代理会借用路径字符串：用一份**独立的 String**，
        // 否则 `ctx_path` 在函数末尾要 move 进 `Self` 时会被认为仍被借用。
        let ctx_dest: String = ctx_path.as_str().to_string();
        let ctx = Proxy::new(conn.as_ref(), SERVICE, ctx_dest.as_str(), IFACE)
            .map_err(|e| format!("IBus 上下文代理失败：{e}"))?;

        // 能力位必须先设，否则 FocusIn 会报 "does not support focus"。
        // `SetCapabilities`/`SetEngine`/`FocusIn` 失败都**不致命**：
        // 上下文仍能处理按键（实测 ProcessKeyEvent 可用），最多是没有候选窗/焦点语义。
        let _ = ctx.call::<_, _, ()>("SetCapabilities", &(CAP_PREEDIT | CAP_FOCUS));
        if !engine.is_empty() {
            let _ = ctx.call::<_, _, ()>("SetEngine", &(engine,));
        }
        let _ = ctx.call::<_, _, ()>("FocusIn", &());

        // 信号线程：IBus 的 CommitText / UpdatePreeditText → mpsc。
        let (tx, rx) = channel::<ImeEvent>();
        let thread_conn = Arc::clone(&conn);
        let ctx_str: String = ctx_path.as_str().to_string();
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .interface(IFACE)
            .map_err(|e| format!("匹配规则构造失败：{e}"))?
            .path(ctx_str.clone())
            .map_err(|e| format!("匹配规则路径失败：{e}"))?
            .build();
        let thread = std::thread::Builder::new()
            .name("y901-ibus".to_string())
            .spawn(move || signal_loop(thread_conn, rule, tx))
            .map_err(|e| format!("IBus 信号线程启动失败：{e}"))?;

        Ok(Self {
            conn,
            ctx_path,
            rx,
            _thread: thread,
        })
    }

    /// 上下文对象路径（日志用）。
    pub fn context_path(&self) -> &str {
        self.ctx_path.as_str()
    }

    fn proxy(&self) -> Option<Proxy<'_>> {
        Proxy::new(self.conn.as_ref(), SERVICE, self.ctx_path.as_str(), IFACE).ok()
    }
}

impl ImeClient for IbusIme {
    fn available(&self) -> bool {
        true
    }

    fn filter_key(&mut self, keyval: u32, keycode: u32, state: u32) -> KeyOutcome {
        let Some(ctx) = self.proxy() else {
            return KeyOutcome::Forward;
        };
        // IBus 返回 true = "已处理，不要再发给应用"。
        match ctx.call::<_, _, bool>("ProcessKeyEvent", &(keyval, keycode, state)) {
            Ok(true) => KeyOutcome::Consumed,
            // false 或出错都放行：宁可英文照常输入，也不要吞键。
            _ => KeyOutcome::Forward,
        }
    }

    fn set_cursor_location(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if let Some(ctx) = self.proxy() {
            let _ = ctx.call::<_, _, ()>("SetCursorLocation", &(x, y, w, h));
        }
    }

    fn focus_in(&mut self) {
        if let Some(ctx) = self.proxy() {
            // 部分 IBus 版本会回 "does not support focus"，忽略即可。
            let _ = ctx.call::<_, _, ()>("FocusIn", &());
        }
    }

    fn focus_out(&mut self) {
        if let Some(ctx) = self.proxy() {
            let _ = ctx.call::<_, _, ()>("FocusOut", &());
        }
    }

    fn take_events(&mut self) -> Vec<ImeEvent> {
        drain(&self.rx)
    }
}

/// 信号线程：把 IBus 的 `CommitText` / `UpdatePreeditText` 转成 [`ImeEvent`]。
///
/// 用 `MessageIterator` 逐条读（阻塞等待），连接断开时返回 `Err` 自然退出。
fn signal_loop(conn: Arc<Connection>, rule: MatchRule<'static>, tx: Sender<ImeEvent>) {
    let Ok(iter) = MessageIterator::for_match_rule(rule, conn.as_ref(), None) else {
        return;
    };
    for msg in iter {
        let Ok(msg) = msg else { break };
        let header = msg.header();
        let Some(member) = header.member() else { continue };
        match member.as_str() {
            "CommitText" => {
                if let Some(text) = extract_signal_text(&msg) {
                    if !text.is_empty() && tx.send(ImeEvent::Commit(text)).is_err() {
                        break;
                    }
                }
            }
            "UpdatePreeditText" => {
                if let Some(text) = extract_signal_text(&msg) {
                    if tx.send(ImeEvent::Preedit(text)).is_err() {
                        break;
                    }
                }
            }
            "HidePreeditText" => {
                if tx.send(ImeEvent::Preedit(String::new())).is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}

/// 从信号 body 里取出 IBus 文本对象中的字符串。
///
/// `CommitText(v)` / `UpdatePreeditText(v, u, b)` 的第一个参数是 variant 包裹的 IBus 对象；
/// 我们递归找**第一个非空字符串**（即 `IBusText.text`）。
fn extract_signal_text(msg: &zbus::Message) -> Option<String> {
    let body = msg.body();
    let value: Value<'_> = body.deserialize().ok()?;
    find_first_string(&value)
}

/// 递归扫描 `Value`，返回第一个非空字符串。
///
/// 要跳过 IBus 的**类名**：序列化对象的第一个字段是 `"IBusText"` /
/// `"IBusAttrList"` 这种类名，它不是要上屏的文本。实测信号形式：
/// `('IBusText', {}, '你好', ('IBusAttrList', {}, []))` ⇒ 真正的文本是第 3 个字段。
fn find_first_string(v: &Value<'_>) -> Option<String> {
    match v {
        Value::Str(s) => {
            let s = s.as_str();
            if s.is_empty() || s.starts_with("IBus") {
                None
            } else {
                Some(s.to_string())
            }
        }
        Value::Value(inner) => find_first_string(inner),
        Value::Structure(st) => st.fields().iter().find_map(find_first_string),
        Value::Array(arr) => arr.iter().find_map(find_first_string),
        Value::Dict(d) => d.iter().find_map(|(k, val)| {
            find_first_string(k).or_else(|| find_first_string(val))
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::find_first_string;
    use zbus::zvariant::{StructureBuilder, Value};

    /// IBus 的文本对象形如 `('IBusText', {}, '你好', ('IBusAttrList', {}, []))`，
    /// `find_first_string` 必须按结构顺序找到第一个非空串（跳过空 properties 字典）。
    #[test]
    fn finds_text_inside_ibus_object_variant() {
        let attrs = StructureBuilder::new()
            .add_field(Value::from("IBusAttrList"))
            .build()
            .unwrap();
        let text_obj = StructureBuilder::new()
            .add_field(Value::from("IBusText"))
            .add_field(Value::from(""))
            .add_field(Value::from("\u{4f60}\u{597d}"))
            .add_field(Value::from(attrs))
            .build()
            .unwrap();
        let wrapped = Value::from(text_obj);
        assert_eq!(
            find_first_string(&wrapped),
            Some("\u{4f60}\u{597d}".to_string())
        );
    }

    /// 空文本（组字被取消 / `HidePreeditText`）应返回 None，而不是空串误提交。
    #[test]
    fn empty_text_yields_none() {
        let obj = StructureBuilder::new()
            .add_field(Value::from("IBusText"))
            .add_field(Value::from(""))
            .build()
            .unwrap();
        assert_eq!(find_first_string(&Value::from(obj)), None);
    }
}
