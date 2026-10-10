//! A tiny JSON writer that formats exactly like System.Text.Json with
//! indentation (as used by DLC Builder), so manifests diff cleanly against
//! official tools: integral numbers have no ".0", float32 values use their
//! shortest form, non-ASCII is written as is.

#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Obj(Vec<(String, J)>),
    Arr(Vec<J>),
    Str(String),
    Int(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
}

impl J {
    pub fn obj() -> J {
        J::Obj(Vec::new())
    }

    /// Appends a field (builder style).
    pub fn with(mut self, k: &str, v: impl Into<J>) -> J {
        if let J::Obj(fields) = &mut self {
            fields.push((k.to_string(), v.into()));
        }
        self
    }

    pub fn to_string_pretty(&self) -> String {
        let mut s = String::new();
        write(self, 0, &mut s);
        s
    }

    /// Converts parsed JSON (e.g. an embedded resource) into a J tree.
    pub fn from_serde(v: &serde_json::Value) -> J {
        match v {
            serde_json::Value::Null => J::Str(String::new()),
            serde_json::Value::Bool(b) => J::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    J::Int(i)
                } else {
                    J::F64(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => J::Str(s.clone()),
            serde_json::Value::Array(a) => J::Arr(a.iter().map(J::from_serde).collect()),
            serde_json::Value::Object(o) => J::Obj(o.iter().map(|(k, v)| (k.clone(), J::from_serde(v))).collect()),
        }
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(impl From<$t> for J { fn from(v: $t) -> J { J::Int(v as i64) } })*};
}
from_int!(i8, u8, i16, u16, i32, u32, i64, usize);

impl From<f32> for J {
    fn from(v: f32) -> J {
        J::F32(v)
    }
}
impl From<f64> for J {
    fn from(v: f64) -> J {
        J::F64(v)
    }
}
impl From<bool> for J {
    fn from(v: bool) -> J {
        J::Bool(v)
    }
}
impl From<&str> for J {
    fn from(v: &str) -> J {
        J::Str(v.to_string())
    }
}
impl From<String> for J {
    fn from(v: String) -> J {
        J::Str(v)
    }
}
impl<T: Into<J>> From<Vec<T>> for J {
    fn from(v: Vec<T>) -> J {
        J::Arr(v.into_iter().map(Into::into).collect())
    }
}

fn indent(n: usize, s: &mut String) {
    for _ in 0..n {
        s.push_str("  ");
    }
}

fn escape(v: &str, s: &mut String) {
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            '\u{08}' => s.push_str("\\b"),
            '\u{0C}' => s.push_str("\\f"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04X}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
}

/// Shortest round-trip representation, no trailing ".0" (Rust's Display does both).
fn number<T: std::fmt::Display>(v: T) -> String {
    format!("{v}")
}

fn write(v: &J, depth: usize, s: &mut String) {
    match v {
        J::Obj(fields) if fields.is_empty() => s.push_str("{}"),
        J::Obj(fields) => {
            s.push_str("{\n");
            for (i, (k, v)) in fields.iter().enumerate() {
                indent(depth + 1, s);
                escape(k, s);
                s.push_str(": ");
                write(v, depth + 1, s);
                if i + 1 < fields.len() {
                    s.push(',');
                }
                s.push('\n');
            }
            indent(depth, s);
            s.push('}');
        }
        J::Arr(items) if items.is_empty() => s.push_str("[]"),
        J::Arr(items) => {
            s.push_str("[\n");
            for (i, v) in items.iter().enumerate() {
                indent(depth + 1, s);
                write(v, depth + 1, s);
                if i + 1 < items.len() {
                    s.push(',');
                }
                s.push('\n');
            }
            indent(depth, s);
            s.push(']');
        }
        J::Str(v) => escape(v, s),
        J::Int(v) => s.push_str(&v.to_string()),
        J::F32(v) => s.push_str(&number(v)),
        J::F64(v) => s.push_str(&number(v)),
        J::Bool(v) => s.push_str(if *v { "true" } else { "false" }),
    }
}
