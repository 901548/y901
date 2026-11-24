//! **JSON 入口的健壮性**（RPC 面对的是外部输入）。
//!
//! 三个真实缺陷，全部在 `y901-ui/src/json.rs`：
//! 1. 递归下降没有深度上限 —— `[[[[…` 打穿栈，栈溢出是 abort（不可捕获），整进程死；
//! 2. `\uD83D\uDE00`（代理对）只取低 16 位，BMP 之外的字符被**静默丢弃**，
//!    Python/JS 默认序列化正是这种写法；
//! 3. `Json::Num` 序列化把超大浮点饱和成 `i64::MAX`，非有限值输出 `inf`/`NaN`
//!    这类非法 JSON，`dump → parse` 往返失败。

use y901_ui::json::Json;

#[test]
fn deeply_nested_input_is_rejected_not_overflowed() {
    // 20 000 层嵌套：旧实现会栈溢出 abort；现在必须是 Err。
    let deep = format!("{}{}", "[".repeat(20_000), "]".repeat(20_000));
    let r = Json::parse(&deep);
    assert!(r.is_err(), "超深嵌套必须返回 Err（不能 abort）");
    let msg = r.unwrap_err();
    assert!(
        msg.contains("nesting") || msg.contains("deep"),
        "错误信息应指出是嵌套过深，实际：{msg}"
    );
}

#[test]
fn nesting_within_the_limit_still_parses() {
    let ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
    assert!(Json::parse(&ok).is_ok(), "100 层嵌套应正常解析");
}

#[test]
fn surrogate_pairs_decode_to_one_char() {
    // U+1F600 😀 = \uD83D \uDE00
    let v = Json::parse(r#""\ud83d\ude00""#).expect("应解析成功");
    assert_eq!(v.as_str(), Some("😀"), "代理对必须合成一个字符");

    // 孤立高位代理必须是错误，而不是静默丢弃
    assert!(
        Json::parse(r#""\ud83d""#).is_err(),
        "孤立高位代理应报错（静默丢字符会让数据无声损坏）"
    );
}

#[test]
fn bmp_escapes_still_work() {
    let v = Json::parse(r#""\u4e2d\u6587""#).expect("应解析成功");
    assert_eq!(v.as_str(), Some("中文"));
}

#[test]
fn numbers_round_trip_and_non_finite_is_rejected() {
    // 普通数字往返
    for n in [0.0f64, 1.0, -2.5, 1e18] {
        let s = Json::Num(n).dump();
        let back = Json::parse(&s).expect("dump 出来的必须是合法 JSON");
        let got = back.as_f64().expect("应是数字");
        assert!(
            (got - n).abs() <= n.abs() * 1e-12 + 1e-12,
            "{n} 往返变成 {got}"
        );
    }

    // 非有限值不能输出 `inf`/`NaN`（那是非法 JSON）
    for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let s = Json::Num(bad).dump();
        assert!(
            Json::parse(&s).is_ok(),
            "非有限数字 {bad} 被序列化成 {s}，不是合法 JSON"
        );
    }
}
