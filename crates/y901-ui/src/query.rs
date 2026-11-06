//! 极简查询语言：让外部工具不必拉全树。
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
    /// 结果条数上限；`None` = 不限。上限存在的理由是外部工具一次拿几千条没有意义。
    pub limit: Option<usize>,
}

impl Query {
    /// 解析选择器字符串，例如 `role=button & label~="Save" & limit=5`。
    ///
    /// 语法：`&` 分隔条件；`=` 精确相等；`~=` 子串包含；值两侧引号会被剥掉。
    /// 合法键：`id` / `role` / `label` / `value` / `disabled` / `exposed` / `limit`。
    /// 返回：解析后的 `Query`；空串与纯空白得到"匹配一切"的空查询。
    /// 失败：未知键、缺少键、`limit` 非非负整数、无法解析的条件 → `Err(人类可读信息)`。
    /// 副作用：无（纯函数）。
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

    /// 判断单个节点是否命中查询。
    ///
    /// 输入：`id`（节点 id）+ `sem`（**已由框架注入 `role` 的**语义）。
    /// 返回：所有条件同时成立才为 `true`；空查询恒为 `true`。
    /// 副作用：无。
    /// 注意：本方法**不**检查 `exposed_to_ai`——是否把不可见节点纳入结果由调用方决定
    /// （网关侧会过滤，否则外部工具能查到不该看的节点）。
    pub fn matches(&self, id: &str, sem: &Semantics) -> bool {
        self.clauses.iter().all(|c| match c {
            Clause::Eq(k, v) => match *k {
                "id" => id == v,
                "role" => sem.role.0 == v.as_str(),
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
                    "role" => sem.role.0.to_string(),
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
    // 角色由框架注入，组件不必手填
    sem.role = node.role();
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
        let mut s = Semantics::default()
            .label(label)
            .exposed(true)
            .disabled(disabled);
        s.role = crate::widget::Role(Box::leak(role.to_string().into_boxed_str()));
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
