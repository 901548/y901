//! 极简 JSON：框架不能依赖 serde，但 RPC 需要序列化。
//!
//! 只做 RPC 用得到的子集：对象、数组、字符串、数字、布尔、null。

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Json {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj() -> Json {
        Json::Obj(Vec::new())
    }

    /// 写入一个键。对非对象值会自动转成对象，
    /// 这样 `Json::default().set(..)` 也能工作（`Semantics::state` 依赖这点）。
    pub fn set(mut self, key: &str, value: impl Into<Json>) -> Json {
        match self {
            Json::Obj(ref mut map) => map.push((key.to_string(), value.into())),
            _ => {
                self = Json::Obj(vec![(key.to_string(), value.into())]);
            }
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(map) => map.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// 序列化成一行 JSON。
    pub fn dump(&self) -> String {
        let mut out = String::new();
        write_json(self, &mut out);
        out
    }

    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        p.skip_ws();
        let value = p.value()?;
        p.skip_ws();
        if p.pos != p.bytes.len() {
            return Err(format!("trailing bytes at {}", p.pos));
        }
        Ok(value)
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.dump())
    }
}

impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Json::Str(v.to_string())
    }
}

impl From<String> for Json {
    fn from(v: String) -> Self {
        Json::Str(v)
    }
}

impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Json::Bool(v)
    }
}

impl From<f64> for Json {
    fn from(v: f64) -> Self {
        Json::Num(v)
    }
}

impl From<f32> for Json {
    fn from(v: f32) -> Self {
        Json::Num(v as f64)
    }
}

impl From<i32> for Json {
    fn from(v: i32) -> Self {
        Json::Num(v as f64)
    }
}

impl From<u16> for Json {
    fn from(v: u16) -> Self {
        Json::Num(v as f64)
    }
}

impl From<u64> for Json {
    fn from(v: u64) -> Self {
        Json::Num(v as f64)
    }
}

impl From<usize> for Json {
    fn from(v: usize) -> Self {
        Json::Num(v as f64)
    }
}

impl From<u32> for Json {
    fn from(v: u32) -> Self {
        Json::Num(v as f64)
    }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(v: Vec<T>) -> Self {
        Json::Arr(v.into_iter().map(Into::into).collect())
    }
}

fn write_json(value: &Json, out: &mut String) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                let _ = write!(out, "{}", *n as i64);
            } else {
                let _ = write!(out, "{n}");
            }
        }
        Json::Str(s) => write_str(s, out),
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out);
            }
            out.push(']');
        }
        Json::Obj(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(k, out);
                out.push(':');
                write_json(v, out);
            }
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        match self.bump() {
            Some(x) if x == b => Ok(()),
            other => Err(format!("expected '{}', got {:?}", b as char, other)),
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(_) => self.number(),
            None => Err("unexpected end".to_string()),
        }
    }

    fn literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(text.as_bytes()) {
            self.pos += text.len();
            Ok(value)
        } else {
            Err(format!("bad literal at {}", self.pos))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number {text:?}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'b') => out.push('\u{8}'),
                    Some(b'f') => out.push('\u{c}'),
                    Some(b'u') => {
                        let hex = self.take_hex(4)?;
                        if let Some(c) = char::from_u32(hex) {
                            out.push(c);
                        }
                    }
                    other => return Err(format!("bad escape {other:?}")),
                },
                Some(byte) if byte < 0x80 => out.push(byte as char),
                Some(_) => {
                    // 多字节 UTF-8：回退并把整段交给 str
                    self.pos -= 1;
                    let rest =
                        std::str::from_utf8(&self.bytes[self.pos..]).map_err(|e| e.to_string())?;
                    let ch = rest.chars().next().ok_or("bad utf8")?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn take_hex(&mut self, n: usize) -> Result<u32, String> {
        let slice = self
            .bytes
            .get(self.pos..self.pos + n)
            .ok_or("truncated \\u")?;
        let text = std::str::from_utf8(slice).map_err(|e| e.to_string())?;
        self.pos += n;
        u32::from_str_radix(text, 16).map_err(|e| e.to_string())
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(Json::Arr(items)),
                other => return Err(format!("bad array separator {other:?}")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut map = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(map));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value()?;
            map.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(Json::Obj(map)),
                other => return Err(format!("bad object separator {other:?}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let text = r#"{"id":1,"method":"invoke","params":{"id":"panel.list","text":"你好\n"}}"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.get("method").and_then(|m| m.as_str()), Some("invoke"));
        let params = v.get("params").unwrap();
        assert_eq!(params.get("text").and_then(|t| t.as_str()), Some("你好\n"));
        // 序列化后还能解析回来
        let again = Json::parse(&v.to_string()).unwrap();
        assert_eq!(again, v);
    }

    #[test]
    fn escapes_and_arrays() {
        let v = Json::parse(r#"[1, 2.5, true, null, "a\"b"]"#).unwrap();
        match v {
            Json::Arr(items) => {
                assert_eq!(items.len(), 5);
                assert_eq!(items[4].as_str(), Some("a\"b"));
            }
            _ => panic!("expected array"),
        }
    }
}
