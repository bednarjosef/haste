//! Text components on the wire: network NBT (play/configuration) and JSON
//! (status, login disconnect).
//!
//! Both encodings go through `serde_json::Value` so that everything
//! `haste_core::Component` does not model (click/hover events, fonts, shadow
//! colours) survives a round trip in `Component::other`. NBT specifics (from the
//! wiki "Text component format" page): a plain-text component may be a bare
//! String tag; style flags are bytes; a list is a component whose first element
//! is the parent and the rest its `extra`; `with` arguments may be numbers.

use haste_core::Component;
use serde_json::{Map, Number, Value};

use super::{DecodeError, DecodeResult};
use crate::nbt::{NbtCompound, NbtTag};

/// Keys whose NBT byte value means a JSON boolean.
const BOOL_KEYS: &[&str] = &[
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "interpret",
];

/// Decodes a component from its network NBT form.
pub fn from_nbt(tag: &NbtTag) -> DecodeResult<Component> {
    from_value(nbt_to_json(tag, None))
}

/// Decodes a component from JSON text.
pub fn from_json_str(s: &str) -> DecodeResult<Component> {
    let v: Value = serde_json::from_str(s)?;
    from_value(v)
}

/// Converts any JSON component form (string, array, object) to `Component`.
pub fn from_value(v: Value) -> DecodeResult<Component> {
    let v = normalize(v);
    let mut c: Component = serde_json::from_value(v).map_err(DecodeError::Json)?;
    strip_content_keys(&mut c);
    Ok(c)
}

/// `Component` flattens its content enum and an "everything else" map side by
/// side; serde hands content keys to both. Remove the duplicates from `other`.
fn strip_content_keys(c: &mut Component) {
    for k in [
        "text",
        "translate",
        "with",
        "fallback",
        "keybind",
        "score",
        "selector",
        "nbt",
    ] {
        c.other.remove(k);
    }
    for e in &mut c.extra {
        strip_content_keys(e);
    }
    if let haste_core::text::Content::Translate { with, .. } = &mut c.content {
        for a in with {
            strip_content_keys(a);
        }
    }
}

/// Encodes a component to its network NBT form (bare String tag for plain text).
pub fn to_nbt(c: &Component) -> NbtTag {
    let v = serde_json::to_value(c).unwrap_or(Value::Null);
    if let Value::Object(m) = &v {
        if m.len() == 1 {
            if let Some(Value::String(s)) = m.get("text") {
                return NbtTag::String(s.as_str().into());
            }
        }
    }
    json_to_nbt(&v)
}

/// Encodes a component as JSON text.
pub fn to_json_string(c: &Component) -> String {
    serde_json::to_string(c).unwrap_or_else(|_| "\"\"".into())
}

fn normalize(v: Value) -> Value {
    match v {
        Value::String(s) => obj_text(s),
        Value::Number(n) => obj_text(n.to_string()),
        Value::Bool(b) => obj_text(b.to_string()),
        Value::Array(items) => {
            let mut it = items.into_iter().map(normalize);
            let Some(mut first) = it.next() else {
                return obj_text(String::new());
            };
            let rest: Vec<Value> = it.collect();
            if !rest.is_empty() {
                if let Value::Object(m) = &mut first {
                    let extra = m.entry("extra").or_insert_with(|| Value::Array(Vec::new()));
                    if let Value::Array(e) = extra {
                        e.extend(rest);
                    }
                }
            }
            first
        }
        Value::Object(mut m) => {
            for key in ["extra", "with"] {
                if let Some(Value::Array(items)) = m.remove(key) {
                    m.insert(
                        key.into(),
                        Value::Array(items.into_iter().map(normalize).collect()),
                    );
                }
            }
            // A component without any content key is empty text.
            let has_content = ["text", "translate", "keybind", "score", "selector", "nbt"]
                .iter()
                .any(|k| m.contains_key(*k));
            if !has_content {
                m.insert("text".into(), Value::String(String::new()));
            }
            Value::Object(m)
        }
        Value::Null => obj_text(String::new()),
    }
}

fn obj_text(s: String) -> Value {
    let mut m = Map::new();
    m.insert("text".into(), Value::String(s));
    Value::Object(m)
}

/// NBT to JSON. `key` is the compound key this value sits under (for the
/// byte-means-boolean rule).
pub fn nbt_to_json(tag: &NbtTag, key: Option<&str>) -> Value {
    match tag {
        NbtTag::End => Value::Null,
        NbtTag::Byte(b) => {
            if key.is_some_and(|k| BOOL_KEYS.contains(&k)) {
                Value::Bool(*b != 0)
            } else {
                Value::Number((*b).into())
            }
        }
        NbtTag::Short(v) => Value::Number((*v).into()),
        NbtTag::Int(v) => Value::Number((*v).into()),
        NbtTag::Long(v) => Value::Number((*v).into()),
        NbtTag::Float(v) => Number::from_f64(*v as f64).map_or(Value::Null, Value::Number),
        NbtTag::Double(v) => Number::from_f64(*v).map_or(Value::Null, Value::Number),
        NbtTag::ByteArray(a) => {
            Value::Array(a.iter().map(|b| Value::Number((*b).into())).collect())
        }
        NbtTag::String(s) => Value::String(s.to_string()),
        NbtTag::List(l) => Value::Array(l.iter().map(|t| unwrap_list_element(t, key)).collect()),
        NbtTag::Compound(c) => compound_to_json(c),
        NbtTag::IntArray(a) => Value::Array(a.iter().map(|v| Value::Number((*v).into())).collect()),
        NbtTag::LongArray(a) => {
            Value::Array(a.iter().map(|v| Value::Number((*v).into())).collect())
        }
    }
}

/// Heterogeneous NBT lists wrap elements in `{"": value}` compounds.
fn unwrap_list_element(t: &NbtTag, key: Option<&str>) -> Value {
    if let NbtTag::Compound(c) = t {
        if c.child_tags.len() == 1 {
            if let Some(inner) = c.child_tags.get("") {
                return nbt_to_json(inner, key);
            }
        }
    }
    nbt_to_json(t, key)
}

fn compound_to_json(c: &NbtCompound) -> Value {
    let mut m = Map::new();
    for (k, v) in &c.child_tags {
        m.insert(k.to_string(), nbt_to_json(v, Some(k)));
    }
    Value::Object(m)
}

/// JSON to NBT (numbers: integers as Int, or Long when out of range; other
/// numbers as Double; booleans as Byte).
pub fn json_to_nbt(v: &Value) -> NbtTag {
    match v {
        Value::Null => NbtTag::Compound(NbtCompound::new()),
        Value::Bool(b) => NbtTag::Byte(*b as i8),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                match i32::try_from(i) {
                    Ok(i) => NbtTag::Int(i),
                    Err(_) => NbtTag::Long(i),
                }
            } else {
                NbtTag::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => NbtTag::String(s.as_str().into()),
        Value::Array(items) => {
            let tags: Vec<NbtTag> = items.iter().map(json_to_nbt).collect();
            let same = tags
                .windows(2)
                .all(|w| w[0].get_type_id() == w[1].get_type_id());
            if same {
                NbtTag::List(tags)
            } else {
                NbtTag::List(
                    tags.into_iter()
                        .map(|t| {
                            let mut c = NbtCompound::new();
                            c.put("", t);
                            NbtTag::Compound(c)
                        })
                        .collect(),
                )
            }
        }
        Value::Object(m) => {
            let mut c = NbtCompound::new();
            for (k, v) in m {
                c.put(k, json_to_nbt(v));
            }
            NbtTag::Compound(c)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::text::Content;

    #[test]
    fn plain_text_is_a_string_tag() {
        let c = Component::text("hi");
        let t = to_nbt(&c);
        assert_eq!(t, NbtTag::String("hi".into()));
        assert_eq!(from_nbt(&t).unwrap(), c);
    }

    #[test]
    fn styled_translate_round_trip_nbt_and_json() {
        let mut c = Component::translate(
            "chat.type.text",
            vec![Component::text("Steve"), Component::text("hello")],
        );
        c.color = Some("yellow".into());
        c.bold = Some(true);
        c.extra.push(Component::text("!"));
        c.other
            .insert("insertion".into(), Value::String("x".into()));
        let t = to_nbt(&c);
        let back = from_nbt(&t).unwrap();
        assert_eq!(back, c);
        let j = to_json_string(&c);
        assert_eq!(from_json_str(&j).unwrap(), c);
    }

    #[test]
    fn nbt_forms_from_the_wiki() {
        // {"type":"text","text":"a","bold":1b,"extra":["b", {"text":"c"}]}
        let mut root = NbtCompound::new();
        root.put_string("type", "text".into());
        root.put_string("text", "a".into());
        root.put_byte("bold", 1);
        let mut c2 = NbtCompound::new();
        c2.put_string("text", "c".into());
        let mut w1 = NbtCompound::new();
        w1.put("", NbtTag::String("b".into()));
        root.put_list("extra", vec![NbtTag::Compound(w1), NbtTag::Compound(c2)]);
        let c = from_nbt(&NbtTag::Compound(root)).unwrap();
        assert_eq!(c.content, Content::Text { text: "a".into() });
        assert_eq!(c.bold, Some(true));
        assert_eq!(c.extra.len(), 2);
        assert_eq!(c.to_plain(&|_| None), "abc");
        // translate with a numeric argument
        let j = r#"{"translate":"x %s","with":[3]}"#;
        assert_eq!(from_json_str(j).unwrap().to_plain(&|_| None), "x 3");
        // bare JSON string and array forms
        assert_eq!(from_json_str("\"hey\"").unwrap(), Component::text("hey"));
        assert_eq!(
            from_json_str(r#"["a","b"]"#).unwrap().to_plain(&|_| None),
            "ab"
        );
    }
}
