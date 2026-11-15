//! JSON built-in.

use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::collections::HashSet;
use std::rc::Rc;

impl Interp {
    pub fn install_json(&mut self) {
        let json = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        json.define_fixed(
            "Symbol.toStringTag",
            Value::String(JsString::from_str("JSON")),
        );
        self.realm
            .global
            .put_hidden("JSON", Value::Object(json.clone()));
        self.define_method(&json, "parse", 2, |interp, _this, args| {
            let text = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let parsed: serde_json::Value =
                serde_json::from_str(&text.to_string_lossy()).map_err(|_| {
                    Completion::Throw(interp.realm.realm_error(
                        &interp.realm.intrinsics.syntax_error_prototype.clone(),
                        "Unexpected token in JSON",
                    ))
                })?;
            let mut value = json_to_value(interp, &parsed)?;
            let reviver = args.get(1).cloned().unwrap_or(Value::Undefined);
            if interp.is_callable_value(&reviver) {
                let holder =
                    JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
                interp.create_data_property(&holder, PropertyKey::from_str(""), value)?;
                value = interp.json_revive(&reviver, &holder, &JsString::empty())?;
            }
            Ok(value)
        });
        self.define_method(&json, "stringify", 3, |interp, _this, args| {
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let replacer = args.get(1).cloned().unwrap_or(Value::Undefined);
            let space = args.get(2).cloned().unwrap_or(Value::Undefined);
            json_stringify(interp, &value, &replacer, &space)
        });
    }

    fn json_revive(
        &mut self,
        reviver: &Value,
        holder: &JsObject,
        key: &JsString,
    ) -> JsResult<Value> {
        let value = self.get_property(
            &Value::Object(holder.clone()),
            &PropertyKey::String(key.clone()),
        )?;
        if let Some(obj) = value.as_object_cloned() {
            let keys = self.own_property_keys(&obj)?;
            for k in keys {
                if let PropertyKey::String(ks) = &k {
                    let new_v = self.json_revive(reviver, &obj, ks)?;
                    if new_v.is_undefined() {
                        self.delete_property(&obj, &k)?;
                    } else {
                        self.set_property_object(
                            &obj,
                            k,
                            new_v,
                            &Value::Object(obj.clone()),
                            true,
                        )?;
                    }
                }
            }
        }
        self.call_function(
            reviver,
            &Value::Object(holder.clone()),
            &[Value::String(key.clone()), value],
        )
    }
}

fn json_to_value(interp: &mut Interp, value: &serde_json::Value) -> JsResult<Value> {
    Ok(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Value::String(JsString::from_str(s)),
        serde_json::Value::Array(arr) => {
            let mut vals = Vec::with_capacity(arr.len());
            for v in arr {
                vals.push(json_to_value(interp, v)?);
            }
            Value::Object(interp.create_array_from_list(vals)?)
        }
        serde_json::Value::Object(map) => {
            let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
            for (k, v) in map {
                let val = json_to_value(interp, v)?;
                interp.create_data_property(&obj, PropertyKey::from_str(k), val)?;
            }
            Value::Object(obj)
        }
    })
}

fn json_stringify(
    interp: &mut Interp,
    value: &Value,
    replacer: &Value,
    space: &Value,
) -> JsResult<Value> {
    let mut gap = String::new();
    if let Value::Number(n) = space {
        let count = (*n as i64).clamp(0, 10) as usize;
        gap = " ".repeat(count);
    } else if let Value::String(s) = space {
        gap = s.to_string_lossy().chars().take(10).collect();
    }
    let mut property_list: Option<Vec<JsString>> = None;
    if let Some(arr) = replacer.as_object_cloned() {
        if interp.is_array(&Value::Object(arr.clone())) {
            let len = interp.length_of_array_like(&arr)? as usize;
            let mut list = Vec::new();
            for i in 0..len {
                let v = interp.get_property(
                    &Value::Object(arr.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                let key = match v {
                    Value::String(s) => Some(s),
                    Value::Number(n) => Some(JsString::from_str(&format_number(n))),
                    _ => None,
                };
                if let Some(k) = key {
                    if !list.contains(&k) {
                        list.push(k);
                    }
                }
            }
            property_list = Some(list);
        }
    }
    let replacer_fn = if interp.is_callable_value(replacer) {
        Some(replacer.clone())
    } else {
        None
    };
    let holder = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
    interp.create_data_property(&holder, PropertyKey::from_str(""), value.clone())?;
    let mut stack: HashSet<usize> = HashSet::new();
    let out = stringify_value(
        interp,
        &Value::Object(holder),
        "",
        &gap,
        "",
        &replacer_fn,
        &property_list,
        &mut stack,
    )?;
    match out {
        Some(s) => Ok(Value::String(JsString::from_str(&s))),
        None => Ok(Value::Undefined),
    }
}

#[allow(clippy::too_many_arguments)]
fn stringify_value(
    interp: &mut Interp,
    holder: &Value,
    key: &str,
    gap: &str,
    indent: &str,
    replacer: &Option<Value>,
    property_list: &Option<Vec<JsString>>,
    stack: &mut HashSet<usize>,
) -> JsResult<Option<String>> {
    let mut value = interp.get_property(holder, &PropertyKey::from_str(key))?;
    if let Some(obj) = value.as_object_cloned() {
        let to_json = interp.get_property(
            &Value::Object(obj.clone()),
            &PropertyKey::from_str("toJSON"),
        )?;
        if interp.is_callable_value(&to_json) {
            value = interp.call_function(
                &to_json,
                &Value::Object(obj),
                &[Value::String(JsString::from_str(key))],
            )?;
        }
    }
    if let Some(rf) = replacer {
        value =
            interp.call_function(rf, holder, &[Value::String(JsString::from_str(key)), value])?;
    }
    // Unbox primitive wrappers.
    if let Some(obj) = value.as_object_cloned() {
        let unboxed = {
            let data = obj.borrow();
            match &data.kind {
                ObjectKind::NumberObject(n) => Some(Value::Number(*n)),
                ObjectKind::StringObject(s) => Some(Value::String(s.clone())),
                ObjectKind::BooleanObject(b) => Some(Value::Bool(*b)),
                ObjectKind::BigIntObject(b) => Some(Value::BigInt(b.clone())),
                _ => None,
            }
        };
        if let Some(v) = unboxed {
            value = v;
        }
    }
    match &value {
        Value::Null => Ok(Some("null".to_string())),
        Value::Bool(b) => Ok(Some(b.to_string())),
        Value::Number(n) => {
            if n.is_finite() {
                Ok(Some(format_number(*n)))
            } else {
                Ok(Some("null".to_string()))
            }
        }
        Value::String(s) => Ok(Some(quote_json(s))),
        Value::BigInt(_) => interp.throw_type_error("Do not know how to serialize a BigInt"),
        Value::Undefined | Value::Symbol(_) => Ok(None),
        Value::Object(obj) => {
            if obj.is_callable() {
                return Ok(None);
            }
            let ptr = Rc::as_ptr(&obj.0) as usize;
            if !stack.insert(ptr) {
                return interp.throw_type_error("Converting circular structure to JSON");
            }
            let next_indent = format!("{}{}", indent, gap);
            let result = if matches!(&obj.borrow().kind, ObjectKind::Array(_)) {
                let len = interp.length_of_array_like(&obj)? as usize;
                let mut parts = Vec::with_capacity(len);
                for i in 0..len {
                    let k = i.to_string();
                    let v = stringify_value(
                        interp,
                        &Value::Object(obj.clone()),
                        &k,
                        gap,
                        &next_indent,
                        replacer,
                        property_list,
                        stack,
                    )?;
                    parts.push(v.unwrap_or_else(|| "null".to_string()));
                }
                if parts.is_empty() {
                    "[]".to_string()
                } else if gap.is_empty() {
                    format!("[{}]", parts.join(","))
                } else {
                    format!(
                        "[\n{}{}\n{}]",
                        next_indent,
                        parts.join(&format!(",\n{}", next_indent)),
                        indent
                    )
                }
            } else {
                let mut keys = Vec::new();
                if let Some(list) = property_list {
                    for k in list {
                        if interp.has_property(&obj, &PropertyKey::String(k.clone()))? {
                            keys.push(k.clone());
                        }
                    }
                } else {
                    for k in interp.own_property_keys(&obj)? {
                        if let PropertyKey::String(s) = k {
                            if let Some(p) =
                                interp.get_own_property(&obj, &PropertyKey::String(s.clone()))?
                            {
                                if p.enumerable {
                                    keys.push(s);
                                }
                            }
                        }
                    }
                }
                let mut parts = Vec::new();
                for k in keys {
                    let v = stringify_value(
                        interp,
                        &Value::Object(obj.clone()),
                        &k.to_string_lossy(),
                        gap,
                        &next_indent,
                        replacer,
                        property_list,
                        stack,
                    )?;
                    if let Some(v) = v {
                        if gap.is_empty() {
                            parts.push(format!("{}:{}", quote_json(&k), v));
                        } else {
                            parts.push(format!("{}: {}", quote_json(&k), v));
                        }
                    }
                }
                if parts.is_empty() {
                    "{}".to_string()
                } else if gap.is_empty() {
                    format!("{{{}}}", parts.join(","))
                } else {
                    format!(
                        "{{\n{}{}\n{}}}",
                        next_indent,
                        parts.join(&format!(",\n{}", next_indent)),
                        indent
                    )
                }
            };
            stack.remove(&ptr);
            Ok(Some(result))
        }
    }
}

fn quote_json(s: &JsString) -> String {
    let mut out = String::from("\"");
    let units = s.as_slice();
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        match u {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x00..=0x1F => out.push_str(&format!("\\u{:04x}", u)),
            0xD800..=0xDBFF => {
                if i + 1 < units.len() && (0xDC00..=0xDFFF).contains(&units[i + 1]) {
                    let cp =
                        0x10000 + (((u as u32 - 0xD800) << 10) | (units[i + 1] as u32 - 0xDC00));
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                    }
                    i += 2;
                    continue;
                } else {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            0xDC00..=0xDFFF => out.push_str(&format!("\\u{:04x}", u)),
            _ => {
                if let Some(c) = char::from_u32(u as u32) {
                    out.push(c);
                }
            }
        }
        i += 1;
    }
    out.push('"');
    out
}

fn _unused(_: IndexMap<PropertyKey, Property>) {}
