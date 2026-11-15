//! String built-in methods.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use std::rc::Rc;

impl Interp {
    pub fn install_string_methods(&mut self) {
        let proto = self.realm.intrinsics.string_prototype.clone();

        self.define_method(&proto, "charAt", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let pos = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            if pos < 0.0 || pos >= s.len() as f64 {
                return Ok(Value::String(JsString::empty()));
            }
            Ok(Value::String(JsString::from_utf16(vec![s
                .code_unit_at(pos as usize)
                .unwrap()])))
        });
        self.define_method(&proto, "charCodeAt", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let pos = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            if pos < 0.0 || pos >= s.len() as f64 {
                return Ok(Value::Number(f64::NAN));
            }
            Ok(Value::Number(s.code_unit_at(pos as usize).unwrap() as f64))
        });
        self.define_method(&proto, "codePointAt", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let pos = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            if pos < 0.0 || pos >= s.len() as f64 {
                return Ok(Value::Undefined);
            }
            let idx = pos as usize;
            let first = s.code_unit_at(idx).unwrap();
            if (0xD800..=0xDBFF).contains(&first) && idx + 1 < s.len() {
                let second = s.code_unit_at(idx + 1).unwrap();
                if (0xDC00..=0xDFFF).contains(&second) {
                    let cp = 0x10000 + (((first as u32 - 0xD800) << 10) | (second as u32 - 0xDC00));
                    return Ok(Value::Number(cp as f64));
                }
            }
            Ok(Value::Number(first as f64))
        });
        self.define_method(&proto, "at", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let pos = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            let idx = if pos < 0.0 { s.len() as f64 + pos } else { pos };
            if idx < 0.0 || idx >= s.len() as f64 {
                return Ok(Value::Undefined);
            }
            Ok(Value::String(JsString::from_utf16(vec![s
                .code_unit_at(idx as usize)
                .unwrap()])))
        });
        self.define_method(&proto, "concat", 1, |interp, this, args| {
            let mut s = interp.this_string(this)?;
            for a in args {
                let t = interp.to_string(a)?;
                s = s.concat(&t);
            }
            Ok(Value::String(s))
        });
        self.define_method(&proto, "includes", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let search = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let pos = string_search_start(interp, args.get(1), s.len())?;
            Ok(Value::Bool(
                s.find_subslice(search.as_slice(), pos).is_some(),
            ))
        });
        self.define_method(&proto, "indexOf", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let search = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let pos = string_search_start(interp, args.get(1), s.len())?;
            Ok(Value::Number(
                s.find_subslice(search.as_slice(), pos)
                    .map(|i| i as f64)
                    .unwrap_or(-1.0),
            ))
        });
        self.define_method(&proto, "lastIndexOf", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let search = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let pos = match args.get(1) {
                Some(v) if !v.is_undefined() => {
                    let n = interp.to_number(v)?;
                    if n.is_nan() {
                        s.len()
                    } else {
                        (n.max(0.0) as usize).min(s.len())
                    }
                }
                _ => s.len(),
            };
            Ok(Value::Number(
                s.last_index_of(search.as_slice(), pos)
                    .map(|i| i as f64)
                    .unwrap_or(-1.0),
            ))
        });
        self.define_method(&proto, "startsWith", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let search = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let pos = string_search_start(interp, args.get(1), s.len())?;
            if pos + search.len() > s.len() {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(s.substring(pos, pos + search.len()) == search))
        });
        self.define_method(&proto, "endsWith", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let search = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let end = match args.get(1) {
                Some(v) if !v.is_undefined() => {
                    let n = interp.to_number(v)?;
                    if n.is_nan() {
                        0
                    } else {
                        (n.max(0.0) as usize).min(s.len())
                    }
                }
                _ => s.len(),
            };
            if search.len() > end {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(s.substring(end - search.len(), end) == search))
        });
        self.define_method(&proto, "slice", 2, |interp, this, args| {
            let s = interp.this_string(this)?;
            let len = s.len();
            let start = relative_string_index(interp, args.first(), len, 0)?;
            let end = relative_string_index(interp, args.get(1), len, len)?;
            Ok(Value::String(s.substring(start, end.max(start))))
        });
        self.define_method(&proto, "substring", 2, |interp, this, args| {
            let s = interp.this_string(this)?;
            let len = s.len();
            let mut start = clamp_index(interp, args.first(), len)?;
            let mut end = match args.get(1) {
                Some(v) if !v.is_undefined() => clamp_index(interp, Some(v), len)?,
                _ => len,
            };
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            Ok(Value::String(s.substring(start, end)))
        });
        self.define_method(&proto, "substr", 2, |interp, this, args| {
            let s = interp.this_string(this)?;
            let len = s.len();
            let start = relative_string_index(interp, args.first(), len, 0)?;
            let count = match args.get(1) {
                Some(v) if !v.is_undefined() => {
                    let n = interp.to_integer_or_infinity(v)?;
                    if n <= 0.0 {
                        0
                    } else {
                        n as usize
                    }
                }
                _ => len,
            };
            Ok(Value::String(s.substring(start, (start + count).min(len))))
        });
        self.define_method(&proto, "repeat", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let n = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            if n < 0.0 || n.is_infinite() {
                return interp.throw_range_error("Invalid count value");
            }
            let n = n as usize;
            if s.len().saturating_mul(n) > 100_000_000 {
                return interp.throw_range_error("Result string too large");
            }
            let mut out = JsString::empty();
            for _ in 0..n {
                out = out.concat(&s);
            }
            Ok(Value::String(out))
        });
        self.define_method(&proto, "padStart", 1, |interp, this, args| {
            string_pad(interp, this, args, true)
        });
        self.define_method(&proto, "padEnd", 1, |interp, this, args| {
            string_pad(interp, this, args, false)
        });
        self.define_method(&proto, "trim", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            Ok(Value::String(trim_units(&s, true, true)))
        });
        self.define_method(&proto, "trimStart", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            Ok(Value::String(trim_units(&s, true, false)))
        });
        self.define_method(&proto, "trimEnd", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            Ok(Value::String(trim_units(&s, false, true)))
        });
        self.define_method(&proto, "toLowerCase", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            Ok(Value::String(map_string_chars(&s, |c| {
                c.to_lowercase().collect()
            })))
        });
        self.define_method(&proto, "toUpperCase", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            Ok(Value::String(map_string_chars(&s, |c| {
                c.to_uppercase().collect()
            })))
        });
        self.define_method(&proto, "toLocaleLowerCase", 0, |interp, this, args| {
            let lower = interp.get_property(this, &PropertyKey::from_str("toLowerCase"))?;
            interp.call_function(&lower, this, args)
        });
        self.define_method(&proto, "toLocaleUpperCase", 0, |interp, this, args| {
            let upper = interp.get_property(this, &PropertyKey::from_str("toUpperCase"))?;
            interp.call_function(&upper, this, args)
        });
        self.define_method(&proto, "localeCompare", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let other = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Number(match s.as_slice().cmp(other.as_slice()) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            }))
        });
        self.define_method(&proto, "normalize", 1, |interp, this, args| {
            let s = interp.this_string(this)?;
            let form = match args.first() {
                Some(v) if !v.is_undefined() => interp.to_string(v)?,
                _ => JsString::from_str("NFC"),
            };
            let f = form.to_string_lossy();
            let normalized = match f.as_str() {
                "NFC" => s.to_string_lossy().nfc().collect::<String>(),
                "NFD" => s.to_string_lossy().nfd().collect::<String>(),
                "NFKC" => s.to_string_lossy().nfkc().collect::<String>(),
                "NFKD" => s.to_string_lossy().nfkd().collect::<String>(),
                _ => return interp.throw_range_error("Invalid normalization form"),
            };
            Ok(Value::String(JsString::from_str(&normalized)))
        });
        self.define_method(&proto, "isWellFormed", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            Ok(Value::Bool(is_well_formed(&s)))
        });
        self.define_method(&proto, "toWellFormed", 0, |interp, this, _args| {
            let s = interp.this_string(this)?;
            let mut out = Vec::with_capacity(s.len());
            let mut i = 0;
            while i < s.len() {
                let u = s.code_unit_at(i).unwrap();
                if (0xD800..=0xDBFF).contains(&u) {
                    if let Some(n) = s.code_unit_at(i + 1) {
                        if (0xDC00..=0xDFFF).contains(&n) {
                            out.push(u);
                            out.push(n);
                            i += 2;
                            continue;
                        }
                    }
                    out.push(0xFFFD);
                } else if (0xDC00..=0xDFFF).contains(&u) {
                    out.push(0xFFFD);
                } else {
                    out.push(u);
                }
                i += 1;
            }
            Ok(Value::String(JsString::from_utf16(out)))
        });
        // split
        self.define_method(&proto, "split", 2, |interp, this, args| {
            let s = interp.this_string(this)?;
            let separator = args.first().cloned().unwrap_or(Value::Undefined);
            if let Some(sep_obj) = separator.as_object_cloned() {
                let splitter = interp.get_method(
                    &separator,
                    &PropertyKey::Symbol(interp.realm.intrinsics.symbol_split.clone()),
                )?;
                if let Some(f) = splitter {
                    return interp.call_function(&f, &separator, &[Value::String(s)]);
                }
            }
            let limit = match args.get(1) {
                Some(v) if !v.is_undefined() => interp.to_uint32(v)? as usize,
                _ => u32::MAX as usize,
            };
            if limit == 0 {
                return Ok(Value::Object(interp.create_array_from_list(Vec::new())?));
            }
            if separator.is_undefined() {
                return Ok(Value::Object(
                    interp.create_array_from_list(vec![Value::String(s)])?,
                ));
            }
            let sep = interp.to_string(&separator)?;
            let mut parts = Vec::new();
            if sep.is_empty() {
                for i in 0..s.len() {
                    if parts.len() >= limit {
                        break;
                    }
                    parts.push(Value::String(JsString::from_utf16(vec![s
                        .code_unit_at(i)
                        .unwrap()])));
                }
            } else {
                let mut start = 0;
                while parts.len() + 1 < limit {
                    match s.find_subslice(sep.as_slice(), start) {
                        Some(pos) => {
                            parts.push(Value::String(s.substring(start, pos)));
                            start = pos + sep.len();
                        }
                        None => break,
                    }
                }
                parts.push(Value::String(s.substring(start, s.len())));
            }
            Ok(Value::Object(interp.create_array_from_list(parts)?))
        });
        // replace / replaceAll
        self.define_method(&proto, "replace", 2, |interp, this, args| {
            string_replace(interp, this, args, false)
        });
        self.define_method(&proto, "replaceAll", 2, |interp, this, args| {
            string_replace(interp, this, args, true)
        });
        self.define_method(&proto, "match", 1, |interp, this, args| {
            let pattern = args.first().cloned().unwrap_or(Value::Undefined);
            if let Some(o) = pattern.as_object_cloned() {
                let f = interp.get_method(
                    &pattern,
                    &PropertyKey::Symbol(interp.realm.intrinsics.symbol_match.clone()),
                )?;
                if let Some(f) = f {
                    return interp.call_function(&f, &Value::Object(o), &[this.clone()]);
                }
            }
            let re = interp.construct_regexp_from_pattern(&pattern, &Value::Undefined)?;
            let exec =
                interp.get_property(&Value::Object(re.clone()), &PropertyKey::from_str("exec"))?;
            let _ = exec;
            let s = interp.this_string(this)?;
            interp.regexp_match_value(&re, &s)
        });
        self.define_method(&proto, "search", 1, |interp, this, args| {
            let pattern = args.first().cloned().unwrap_or(Value::Undefined);
            if let Some(o) = pattern.as_object_cloned() {
                let f = interp.get_method(
                    &pattern,
                    &PropertyKey::Symbol(interp.realm.intrinsics.symbol_search.clone()),
                )?;
                if let Some(f) = f {
                    return interp.call_function(&f, &Value::Object(o), &[this.clone()]);
                }
            }
            let re = interp.construct_regexp_from_pattern(&pattern, &Value::Undefined)?;
            let s = interp.this_string(this)?;
            interp.regexp_search_value(&re, &s)
        });
        self.define_method(&proto, "matchAll", 1, |interp, this, args| {
            let pattern = args.first().cloned().unwrap_or(Value::Undefined);
            let re = interp
                .construct_regexp_from_pattern(&pattern, &Value::String(JsString::from_str("g")))?;
            let s = interp.this_string(this)?;
            interp.regexp_match_all(&re, &s)
        });
        // String iterator
        let values = self.native_method("values", 0, move |interp, this, _args| {
            let s = interp.this_string(this)?;
            let index = Rc::new(std::cell::RefCell::new(0usize));
            let s2 = s.clone();
            let next = native_fn("next", 0, move |interp, _this, _args| {
                let mut i = index.borrow_mut();
                if *i >= s2.len() {
                    let obj =
                        JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
                    obj.put_hidden("value", Value::Undefined);
                    obj.put_hidden("done", Value::Bool(true));
                    return Ok(Value::Object(obj));
                }
                let first = s2.code_unit_at(*i).unwrap();
                let mut units = vec![first];
                *i += 1;
                if (0xD800..=0xDBFF).contains(&first) && *i < s2.len() {
                    let second = s2.code_unit_at(*i).unwrap();
                    if (0xDC00..=0xDFFF).contains(&second) {
                        units.push(second);
                        *i += 1;
                    }
                }
                let obj =
                    JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
                obj.put_hidden("value", Value::String(JsString::from_utf16(units)));
                obj.put_hidden("done", Value::Bool(false));
                Ok(Value::Object(obj))
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let iter = JsObject::ordinary(Some(
                interp.realm.intrinsics.string_iterator_prototype.clone(),
            ));
            iter.put_hidden("next", Value::Object(next));
            Ok(Value::Object(iter))
        });
        let values_obj = values.clone();
        let desc = Descriptor {
            value: Some(Value::Object(values_obj)),
            writable: Some(true),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(
            &proto,
            PropertyKey::Symbol(self.realm.intrinsics.symbol_iterator.clone()),
            desc,
        );
    }

    pub fn this_string(&mut self, this: &Value) -> JsResult<JsString> {
        match this {
            Value::String(s) => Ok(s.clone()),
            Value::Object(o) => {
                let v = {
                    let data = o.borrow();
                    match &data.kind {
                        ObjectKind::StringObject(s) => s.clone(),
                        _ => {
                            drop(data);
                            return self
                                .throw_type_error("String method called on incompatible receiver");
                        }
                    }
                };
                Ok(v)
            }
            _ => self.throw_type_error("String method called on incompatible receiver"),
        }
    }

    pub fn construct_regexp_from_pattern(
        &mut self,
        pattern: &Value,
        flags: &Value,
    ) -> JsResult<JsObject> {
        let ctor = self.realm.intrinsics.regexp_ctor.clone();
        match ctor {
            Some(c) => self
                .construct_function(&Value::Object(c), &[pattern.clone(), flags.clone()], None)
                .map(|v| v.as_object_cloned().unwrap()),
            None => self.make_regexp(
                match pattern.as_string() {
                    Some(s) => s.clone(),
                    None => JsString::empty(),
                },
                match flags.as_string() {
                    Some(s) => s.clone(),
                    None => JsString::empty(),
                },
            ),
        }
    }
}

fn string_search_start(interp: &mut Interp, value: Option<&Value>, len: usize) -> JsResult<usize> {
    match value {
        Some(v) if !v.is_undefined() => {
            let n = interp.to_integer_or_infinity(v)?;
            Ok((n.max(0.0) as usize).min(len))
        }
        _ => Ok(0),
    }
}

fn relative_string_index(
    interp: &mut Interp,
    value: Option<&Value>,
    len: usize,
    default: usize,
) -> JsResult<usize> {
    match value {
        Some(v) if !v.is_undefined() => {
            let n = interp.to_integer_or_infinity(v)?;
            if n < 0.0 {
                Ok(((len as f64 + n).max(0.0)) as usize)
            } else {
                Ok((n as usize).min(len))
            }
        }
        _ => Ok(default),
    }
}

fn clamp_index(interp: &mut Interp, value: Option<&Value>, len: usize) -> JsResult<usize> {
    match value {
        Some(v) if !v.is_undefined() => {
            let n = interp.to_integer_or_infinity(v)?;
            Ok(n.clamp(0.0, len as f64) as usize)
        }
        _ => Ok(0),
    }
}

fn string_pad(interp: &mut Interp, this: &Value, args: &[Value], start: bool) -> JsResult<Value> {
    let s = interp.this_string(this)?;
    let max_len = interp.to_length(args.first().unwrap_or(&Value::Undefined))? as usize;
    if max_len <= s.len() {
        return Ok(Value::String(s));
    }
    let filler = match args.get(1) {
        Some(v) if !v.is_undefined() => interp.to_string(v)?,
        _ => JsString::from_str(" "),
    };
    if filler.is_empty() {
        return Ok(Value::String(s));
    }
    let fill_len = max_len - s.len();
    let mut pad = JsString::empty();
    while pad.len() < fill_len {
        pad = pad.concat(&filler);
    }
    let pad = pad.substring(0, fill_len);
    Ok(Value::String(if start {
        pad.concat(&s)
    } else {
        s.concat(&pad)
    }))
}

fn trim_units(s: &JsString, start: bool, end: bool) -> JsString {
    let mut a = 0usize;
    let mut b = s.len();
    if start {
        while a < b {
            let c = char_from_units(s, a);
            if is_whitespace_char(c) {
                a += c.len_utf16();
            } else {
                break;
            }
        }
    }
    if end {
        while b > a {
            let mut found = false;
            for len in 1..=2 {
                if b >= a + len {
                    let candidate = s.substring(b - len, b);
                    if let Ok(st) = candidate.try_to_utf8() {
                        if let Some(c) = st.chars().next() {
                            if c.len_utf16() == len && is_whitespace_char(c) {
                                b -= len;
                                found = true;
                                break;
                            }
                        }
                    }
                }
            }
            if !found {
                break;
            }
        }
    }
    s.substring(a, b)
}

fn char_from_units(s: &JsString, i: usize) -> char {
    let units = &s.as_slice()[i..];
    let mut buf = [0u16; 2];
    let len = if units[0] >= 0xD800 && units[0] <= 0xDBFF && units.len() > 1 {
        buf[0] = units[0];
        buf[1] = units[1];
        2
    } else {
        buf[0] = units[0];
        1
    };
    char::decode_utf16(buf[..len].iter().copied())
        .next()
        .and_then(|r| r.ok())
        .unwrap_or('\u{FFFD}')
}

fn is_whitespace_char(c: char) -> bool {
    crate::ops::is_js_whitespace(c)
}

fn map_string_chars<F: Fn(char) -> String>(s: &JsString, f: F) -> JsString {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let c = char_from_units(s, i);
        out.push_str(&f(c));
        i += c.len_utf16();
    }
    JsString::from_str(&out)
}

fn is_well_formed(s: &JsString) -> bool {
    let mut i = 0;
    while i < s.len() {
        let u = s.code_unit_at(i).unwrap();
        if (0xD800..=0xDBFF).contains(&u) {
            if let Some(n) = s.code_unit_at(i + 1) {
                if (0xDC00..=0xDFFF).contains(&n) {
                    i += 2;
                    continue;
                }
            }
            return false;
        }
        if (0xDC00..=0xDFFF).contains(&u) {
            return false;
        }
        i += 1;
    }
    true
}

fn string_replace(interp: &mut Interp, this: &Value, args: &[Value], all: bool) -> JsResult<Value> {
    let search = args.first().cloned().unwrap_or(Value::Undefined);
    if let Some(o) = search.as_object_cloned() {
        let f = interp.get_method(
            &search,
            &PropertyKey::Symbol(interp.realm.intrinsics.symbol_replace.clone()),
        )?;
        if let Some(f) = f {
            return interp.call_function(
                &f,
                &Value::Object(o),
                &[
                    this.clone(),
                    args.get(1).cloned().unwrap_or(Value::Undefined),
                ],
            );
        }
    }
    let s = interp.this_string(this)?;
    let search_s = interp.to_string(&search)?;
    let replace = args.get(1).cloned().unwrap_or(Value::Undefined);
    let mut out = JsString::empty();
    let mut last = 0usize;
    let mut found_any = false;
    loop {
        let pos = match s.find_subslice(search_s.as_slice(), last) {
            Some(p) => p,
            None => break,
        };
        found_any = true;
        out = out.concat(&s.substring(last, pos));
        let matched = search_s.clone();
        let replacement = if interp.is_callable_value(&replace) {
            let r = interp.call_function(
                &replace,
                &Value::Undefined,
                &[
                    Value::String(matched.clone()),
                    Value::Number(pos as f64),
                    Value::String(s.clone()),
                ],
            )?;
            interp.to_string(&r)?
        } else {
            let r = interp.to_string(&replace)?;
            expand_replacement(&r, &matched, &s, pos)
        };
        out = out.concat(&replacement);
        last = pos + search_s.len();
        if !all {
            break;
        }
        if search_s.is_empty() {
            if last >= s.len() {
                break;
            }
            out = out.concat(&s.substring(last, last + 1));
            last += 1;
        }
    }
    if !all || !found_any {
        out = out.concat(&s.substring(last, s.len()));
    } else {
        out = out.concat(&s.substring(last, s.len()));
    }
    Ok(Value::String(out))
}

fn expand_replacement(
    replacement: &JsString,
    matched: &JsString,
    s: &JsString,
    pos: usize,
) -> JsString {
    let r = replacement.to_string_lossy();
    let mut out = String::new();
    let mut chars = r.chars().peekable();
    let mut index = 0usize;
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            index += 1;
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                out.push('$');
                index += 1;
            }
            Some('&') => {
                chars.next();
                out.push_str(&matched.to_string_lossy());
                index += 1;
            }
            Some('`') => {
                chars.next();
                out.push_str(&s.substring(0, pos).to_string_lossy());
                index += 1;
            }
            Some('\'') => {
                chars.next();
                out.push_str(&s.substring(pos + matched.len(), s.len()).to_string_lossy());
                index += 1;
            }
            _ => {
                out.push('$');
                index += 1;
            }
        }
    }
    let _ = index;
    JsString::from_str(&out)
}

use unicode_normalization::UnicodeNormalization;
