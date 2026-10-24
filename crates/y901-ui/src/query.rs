//! 极简查询语言：让 Agent 不必拉全树。
//!
//! 语法：
//! ```text
//! role=button
//! label~="Save"            // 模糊（包含，大小写不敏感）
//! disabled=false
//! role=button & label~="Save" & limit=5
//! ```
//! 支持的键：`id` / `role` / `label` / `value` / `disabled` / `exposed`
//! 操作符：`=` 精确、`~=` 包含。`limit=N` 是修饰符，不是过滤条件。

use crate::widget::{Semantics, Widget};

/// 一个过滤条件。
#[derive(Clone, Debug, PartialEq)]
enum Clause {
    Eq(&'static str, String),
    Contains(&'static str, String),
}

/// 解析后的查询。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    clauses: Vec<Clause>,
    pub limit: Option<usize>,
}

impl Query {
    /// 解析失败返回 Err（错误信息面向人类，便于调试）。
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut q = Query::default();
        for raw in input.split('&') {
            let part = raw.trim();
            if part.is_empty() {
                continue;
            }
            let (key, op, value) = split_clause(part)?;
            let key_static = match key {
                "id" => "id",
                "role" => "role",
                "label" => "label",
                "value" => "value",
                "disabled" => "disabled",
                "exposed" => "exposed",
                "limit" => {
                    let n = value
                        .parse::<usize>()
                        .map_err(|_| format!("limit 需要非负整数，收到 `{value}`"))?;
                    q.limit = Some(n);
                    continue;
                }
                other => return Err(format!("未知查询键 `{other}`")),
            };
            if op == "~=" {
                q.clauses.push(Clause::Contains(key_static, value));
            } else {
                q.clauses.push(Clause::Eq(key_static, value));
            }
        }
        Ok(q)
    }

    /// 节点是否匹配（只匹配 `exposed_to_ai` 的节点由调用方决定）。
    pub fn matches(&self, id: &str, sem: &Semantics) -> bool {
        self.clauses.iter().all(|c| match c {
            Clause::Eq(k, v) => match *k {
                "id" => id == v,
                "role" => sem.role_name == *v,
                "label" => sem.label == *v,
                "value" => sem.value == *v,
                "disabled" => sem.disabled.to_string() == *v,
                "exposed" => sem.exposed_to_ai.to_string() == *v,
                _ => false,
            },
            Clause::Contains(k, v) => {
                let needle = v.to_lowercase();
                let hay = match *k {
                    "id" => id.to_string(),
                    "role" => sem.role_name.clone(),
                    "label" => sem.label.clone(),
                    "value" => sem.value.clone(),
                    _ => return false,
                }
                .to_lowercase();
                hay.contains(&needle)
            }
        })
    }
}

/// `key=value` / `key~="value"` / `key='value'`。/// 解析 `key=value` / `key~="value"` / `key='value'`。
fn split_clause(part: &str) -> Result<(&str, &'static str, String), String> {
    let (key, op, rest) = if let Some((k, r)) = part.split_once("~=") {
        (k.trim(), "~=", r)
    } else if let Some((k, r)) = part.split_once('=') {
        (k.trim(), "=", r)
    } else {
        return Err(format!("无法解析条件 `{part}`"));
    };
    if key.is_empty() {
        return Err(format!("条件 `{part}` 缺少键"));
    }
    let value = rest
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string();
    Ok((key, op, value))
}
/// 在树上执行查询，返回匹配节点的 id（树序）。
pub fn run(root: &dyn Widget, q: &Query) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, q, &mut out);
    if let Some(n) = q.limit {
        out.truncate(n);
    }
    out
}

fn walk(node: &dyn Widget, q: &Query, out: &mut Vec<String>) {
    if let Some(limit) = q.limit {
        if out.len() >= limit {
            return;
        }
    }
    let mut sem = node.semantics();
    sem.role_name = node.role().as_str().to_string();
    if q.matches(node.id(), &sem) {
        out.push(node.id().to_string());
    }
    for child in node.children() {
        walk(child.as_ref(), q, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sem(role: &str, label: &str, disabled: bool) -> Semantics {
        let mut s = Semantics::default().label(label).exposed(true).disabled(disabled);
        s.role_name = role.to_string();
        s
    }

    #[test]
    fn parse_role_and_label() {
        let q = Query::parse(r#"role=button & label~="Save""#).unwrap();
        assert!(q.matches("b1", &sem("button", "Save file", false)));
        assert!(!q.matches("b2", &sem("button", "Quit", false)));
        assert!(!q.matches("t1", &sem("text_input", "Save file", false)));
    }

    #[test]
    fn parse_bool_and_limit() {
        let q = Query::parse("disabled=true & limit=3").unwrap();
        assert_eq!(q.limit, Some(3));
        assert!(q.matches("x", &sem("button", "", true)));
        assert!(!q.matches("y", &sem("button", "", false)));
    }

    #[test]
    fn unknown_key_is_error() {
        assert!(Query::parse("nope=1").is_err());
        assert!(Query::parse("limit=abc").is_err());
    }

    #[test]
    fn contains_is_case_insensitive() {
        let q = Query::parse(r#"label~="save""#).unwrap();
        assert!(q.matches("b", &sem("button", "SAVE AS", false)));
    }
}
