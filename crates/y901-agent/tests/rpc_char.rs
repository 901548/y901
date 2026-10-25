//! `{"type":"char","code":N}` 的解析边界与回环。
//!
//! 语义：**按一个字符键**（区别于 `RpcEvent::Text` 的"输入一串文本"）。
//! 只接受 `code`（码点整数），拒绝 `text` 别名。

use y901_agent::rpc::RpcEvent;
use y901_ui::{Json, Key};

fn parse(code: Json) -> Result<RpcEvent, String> {
    RpcEvent::from_json(&Json::obj().set("type", "char").set("code", code))
}

fn expect_char(code: u32, want: char) {
    match parse(Json::Num(code as f64)) {
        Ok(RpcEvent::Key(Key::Char(ch))) => assert_eq!(ch, want, "code {code:#x} 解出错误字符"),
        other => panic!("code {code:#x} 应解析为 Char，实际 {other:?}"),
    }
}

fn expect_400(code: Json, why: &str) {
    assert!(
        parse(code.clone()).is_err(),
        "{why}：code={code:?} 应被拒绝，实际通过"
    );
}

#[test]
fn char_accepts_cjk() {
    expect_char(0x4e2d, '中');
    expect_char(0x1f600, '😀'); // 补充平面
    expect_char(0x41, 'A');
}

#[test]
fn char_rejects_high_surrogate() {
    expect_400(Json::Num(0xD800 as f64), "UTF-16 高代理");
}

#[test]
fn char_rejects_low_surrogate() {
    expect_400(Json::Num(0xDFFF as f64), "UTF-16 低代理");
}

#[test]
fn char_rejects_beyond_unicode() {
    expect_400(Json::Num(0x110000 as f64), "超出 Unicode 上限");
}

#[test]
fn char_rejects_u64_max() {
    // u64::MAX 作为 f64 是 1.8e19，超过 u32::MAX
    expect_400(Json::Num(u64::MAX as f64), "u64::MAX 溢出");
}

#[test]
fn char_rejects_negative() {
    expect_400(Json::Num(-1.0), "负数");
}

#[test]
fn char_roundtrips_through_to_json() {
    for ch in ['A', '中', '😀'] {
        let ev = RpcEvent::Key(Key::Char(ch));
        let json = ev.to_json();
        assert_eq!(
            json.get("type").and_then(|v| v.as_str()),
            Some("char"),
            "Char 必须序列化为 type=char，实际 {json}"
        );
        assert_eq!(
            json.get("code").and_then(|v| v.as_f64()),
            Some(ch as u32 as f64),
            "code 必须是码点"
        );
        let back = RpcEvent::from_json(&json).expect("回环解析");
        assert_eq!(back, ev, "{ch:?} 回环不一致");
        assert_eq!(back.to_json(), json, "{ch:?} 二次序列化必须稳定");
    }
}
