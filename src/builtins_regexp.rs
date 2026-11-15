//! RegExp built-in.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::rc::Rc;

impl Interp {
    pub fn install_regexp(&mut self) {
        let proto = self.realm.intrinsics.regexp_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "RegExp",
            2,
            |interp, _this, args| interp.regexp_constructor(args, None),
            |interp, args, new_target| interp.regexp_constructor(args, Some(new_target)),
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("RegExp", Value::Object(ctor.clone()));
        self.realm.intrinsics.regexp_ctor = Some(ctor.clone());

        self.define_method(&proto, "exec", 1, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            interp.regexp_exec(&re, &s)
        });
        self.define_method(&proto, "test", 1, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let r = interp.regexp_exec(&re, &s)?;
            Ok(Value::Bool(!r.is_null()))
        });
        self.define_method(&proto, "toString", 0, |interp, this, _args| {
            let re = interp.regexp_object(this)?;
            let (source, flags) = {
                let data = re.borrow();
                if let ObjectKind::RegExp(r) = &data.kind {
                    (r.source.clone(), r.flags.clone())
                } else {
                    return interp
                        .throw_type_error("RegExp method called on incompatible receiver");
                }
            };
            Ok(Value::String(
                JsString::from_str("/")
                    .concat(&source)
                    .concat(&JsString::from_str("/"))
                    .concat(&flags),
            ))
        });
        // Accessors
        let getter = self.native_method("get source", 0, |interp, this, _args| {
            let re = interp.regexp_object(this)?;
            let data = re.borrow();
            if let ObjectKind::RegExp(r) = &data.kind {
                Ok(Value::String(r.source.clone()))
            } else {
                interp.throw_type_error("RegExp method called on incompatible receiver")
            }
        });
        self.define_accessor_public(&proto, "source", getter.clone(), None);
        let getter = self.native_method("get flags", 0, |interp, this, _args| {
            let re = interp.regexp_object(this)?;
            let data = re.borrow();
            if let ObjectKind::RegExp(r) = &data.kind {
                Ok(Value::String(r.flags.clone()))
            } else {
                interp.throw_type_error("RegExp method called on incompatible receiver")
            }
        });
        self.define_accessor_public(&proto, "flags", getter, None);
        for (name, flag) in [
            ("global", 'g'),
            ("ignoreCase", 'i'),
            ("multiline", 'm'),
            ("dotAll", 's'),
            ("unicode", 'u'),
            ("sticky", 'y'),
            ("hasIndices", 'd'),
            ("unicodeSets", 'v'),
        ] {
            let f = flag;
            let getter =
                self.native_method(&format!("get {}", name), 0, move |interp, this, _args| {
                    let re = interp.regexp_object(this)?;
                    let data = re.borrow();
                    if let ObjectKind::RegExp(r) = &data.kind {
                        Ok(Value::Bool(r.flags.to_string_lossy().contains(f)))
                    } else {
                        interp.throw_type_error("RegExp method called on incompatible receiver")
                    }
                });
            self.define_accessor_public(&proto, name, getter, None);
        }
        // Symbol methods.
        self.define_method(&proto, "@@match", 1, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            interp.regexp_symbol_match(&re, &s)
        });
        let match_fn = self
            .get_property(
                &Value::Object(proto.clone()),
                &PropertyKey::from_str("@@match"),
            )
            .ok();
        if let Some(v) = match_fn {
            let desc = Descriptor {
                value: Some(v),
                writable: Some(true),
                enumerable: Some(false),
                configurable: Some(true),
                ..Default::default()
            };
            let _ = self.define_own_property(
                &proto,
                PropertyKey::Symbol(self.realm.intrinsics.symbol_match.clone()),
                desc,
            );
        }
        self.define_method(&proto, "@@replace", 2, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let replace = args.get(1).cloned().unwrap_or(Value::Undefined);
            interp.regexp_symbol_replace(&re, &s, &replace)
        });
        self.define_symbol_method(
            "@@replace",
            "replace",
            self.realm.intrinsics.symbol_replace.clone(),
        );
        self.define_method(&proto, "@@search", 1, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            interp.regexp_search_value(&re, &s)
        });
        self.define_symbol_method(
            "@@search",
            "search",
            self.realm.intrinsics.symbol_search.clone(),
        );
        self.define_method(&proto, "@@split", 2, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            let limit = match args.get(1) {
                Some(v) if !v.is_undefined() => interp.to_uint32(v)? as usize,
                _ => u32::MAX as usize,
            };
            interp.regexp_split(&re, &s, limit)
        });
        self.define_symbol_method(
            "@@split",
            "split",
            self.realm.intrinsics.symbol_split.clone(),
        );
        self.define_method(&proto, "@@matchAll", 1, |interp, this, args| {
            let re = interp.regexp_object(this)?;
            let s = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            interp.regexp_match_all(&re, &s)
        });
        self.define_symbol_method(
            "@@matchAll",
            "matchAll",
            self.realm.intrinsics.symbol_match_all.clone(),
        );
    }

    fn define_symbol_method(&mut self, method_name: &str, prop_name: &str, sym: SymbolValue) {
        let proto = self.realm.intrinsics.regexp_prototype.clone();
        let value = self
            .get_property(
                &Value::Object(proto.clone()),
                &PropertyKey::from_str(method_name),
            )
            .ok();
        if let Some(v) = value {
            let desc = Descriptor {
                value: Some(v),
                writable: Some(true),
                enumerable: Some(false),
                configurable: Some(true),
                ..Default::default()
            };
            let _ = self.define_own_property(&proto, PropertyKey::Symbol(sym), desc);
        }
        let _ = prop_name;
    }

    pub fn define_accessor_public(
        &mut self,
        obj: &JsObject,
        name: &str,
        get: JsObject,
        set: Option<JsObject>,
    ) {
        let desc = Descriptor {
            get: Some(Some(get)),
            set: Some(set),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(obj, PropertyKey::from_str(name), desc);
    }

    fn regexp_constructor(
        &mut self,
        args: &[Value],
        _new_target: Option<&JsObject>,
    ) -> JsResult<Value> {
        let pattern = args.first().cloned().unwrap_or(Value::Undefined);
        let flags = args.get(1).cloned().unwrap_or(Value::Undefined);
        if let Some(o) = pattern.as_object_cloned() {
            if let ObjectKind::RegExp(data) = &o.borrow().kind {
                if flags.is_undefined() {
                    let source = data.source.clone();
                    let f = data.flags.clone();
                    let re = self.make_regexp(source, f)?;
                    return Ok(Value::Object(re));
                }
            }
        }
        let source = if pattern.is_undefined() {
            JsString::empty()
        } else if let Some(o) = pattern.as_object_cloned() {
            let d = o.borrow();
            if let ObjectKind::RegExp(r) = &d.kind {
                r.source.clone()
            } else {
                drop(d);
                self.to_string(&pattern)?
            }
        } else {
            self.to_string(&pattern)?
        };
        let flag_str = if flags.is_undefined() {
            JsString::empty()
        } else {
            self.to_string(&flags)?
        };
        let re = self.make_regexp(source, flag_str)?;
        Ok(Value::Object(re))
    }

    pub fn regexp_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::RegExp(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("RegExp method called on incompatible receiver")
    }

    fn flags_global(&self, re: &JsObject) -> bool {
        match &re.borrow().kind {
            ObjectKind::RegExp(r) => r.flags.to_string_lossy().contains('g'),
            _ => false,
        }
    }
    fn flags_sticky(&self, re: &JsObject) -> bool {
        match &re.borrow().kind {
            ObjectKind::RegExp(r) => r.flags.to_string_lossy().contains('y'),
            _ => false,
        }
    }
    fn flags_unicode(&self, re: &JsObject) -> bool {
        match &re.borrow().kind {
            ObjectKind::RegExp(r) => {
                let f = r.flags.to_string_lossy();
                f.contains('u') || f.contains('v')
            }
            _ => false,
        }
    }

    fn regexp_match_at(
        &mut self,
        re: &JsObject,
        s: &JsString,
        start: usize,
    ) -> JsResult<Option<regress::Match>> {
        let compact = match &re.borrow().kind {
            ObjectKind::RegExp(r) => r.regex.clone(),
            _ => None,
        };
        let regexp = match compact {
            Some(r) => r,
            None => return Ok(None),
        };
        let unicode = self.flags_unicode(re);
        if unicode {
            Ok(regexp.find_from_utf16(s.as_slice(), start).next())
        } else {
            Ok(regexp.find_from_ucs2(s.as_slice(), start).next())
        }
    }

    pub fn regexp_exec(&mut self, re: &JsObject, s: &JsString) -> JsResult<Value> {
        let global = self.flags_global(re) || self.flags_sticky(re);
        let last_index = if global {
            let v = self.get_property(
                &Value::Object(re.clone()),
                &PropertyKey::from_str("lastIndex"),
            )?;
            self.to_length(&v)?
        } else {
            0.0
        };
        if last_index > s.len() as f64 {
            if global {
                self.set_property_object(
                    re,
                    PropertyKey::from_str("lastIndex"),
                    Value::Number(0.0),
                    &Value::Object(re.clone()),
                    true,
                )?;
            }
            return Ok(Value::Null);
        }
        let start = last_index as usize;
        let m = match self.regexp_match_at(re, s, start)? {
            Some(m) => m,
            None => {
                if global {
                    self.set_property_object(
                        re,
                        PropertyKey::from_str("lastIndex"),
                        Value::Number(0.0),
                        &Value::Object(re.clone()),
                        true,
                    )?;
                }
                return Ok(Value::Null);
            }
        };
        if self.flags_sticky(re) && m.start() != start {
            if global {
                self.set_property_object(
                    re,
                    PropertyKey::from_str("lastIndex"),
                    Value::Number(0.0),
                    &Value::Object(re.clone()),
                    true,
                )?;
            }
            return Ok(Value::Null);
        }
        if global {
            self.set_property_object(
                re,
                PropertyKey::from_str("lastIndex"),
                Value::Number(m.end() as f64),
                &Value::Object(re.clone()),
                true,
            )?;
        }
        let arr = JsObject::array(Some(self.realm.intrinsics.array_prototype.clone()));
        let mut groups = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        let mut has_groups = false;
        let group_count = m.captures.len() + 1;
        for i in 0..group_count.max(1) {
            let v = match m.group(i) {
                Some(r) => Value::String(s.substring(r.start, r.end)),
                None => Value::Undefined,
            };
            arr.put(PropertyKey::String(JsString::from_str(&i.to_string())), v);
        }
        if let ObjectKind::Array(a) = &mut arr.0.borrow_mut().kind {
            a.length = group_count.max(1) as u32;
        }
        // Named groups.
        for (name, range) in m.named_groups() {
            let name = JsString::from_str(name);
            let v = match range {
                Some(r) => Value::String(s.substring(r.start, r.end)),
                None => Value::Undefined,
            };
            groups.put(name, v);
            has_groups = true;
        }
        arr.put_hidden("index", Value::Number(m.start() as f64));
        arr.put_hidden("input", Value::String(s.clone()));
        if has_groups {
            arr.put_hidden("groups", Value::Object(groups));
        } else {
            arr.put_hidden("groups", Value::Undefined);
        }
        Ok(Value::Object(arr))
    }

    pub fn regexp_match_value(&mut self, re: &JsObject, s: &JsString) -> JsResult<Value> {
        if self.flags_global(re) {
            return self.regexp_symbol_match(re, s);
        }
        self.regexp_exec(re, s)
    }

    pub fn regexp_symbol_match(&mut self, re: &JsObject, s: &JsString) -> JsResult<Value> {
        let global = self.flags_global(re);
        if global {
            self.set_property_object(
                re,
                PropertyKey::from_str("lastIndex"),
                Value::Number(0.0),
                &Value::Object(re.clone()),
                true,
            )?;
        }
        let mut results = Vec::new();
        loop {
            let r = self.regexp_exec(re, s)?;
            if r.is_null() {
                break;
            }
            if global {
                let obj = r.as_object_cloned().unwrap();
                let m =
                    self.get_property(&Value::Object(obj.clone()), &PropertyKey::from_str("0"))?;
                results.push(m);
            } else {
                return Ok(r);
            }
        }
        if results.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(Value::Object(self.create_array_from_list(results)?))
        }
    }

    pub fn regexp_search_value(&mut self, re: &JsObject, s: &JsString) -> JsResult<Value> {
        let saved = self.get_property(
            &Value::Object(re.clone()),
            &PropertyKey::from_str("lastIndex"),
        )?;
        if self.flags_global(re) || self.flags_sticky(re) {
            self.set_property_object(
                re,
                PropertyKey::from_str("lastIndex"),
                Value::Number(0.0),
                &Value::Object(re.clone()),
                true,
            )?;
        }
        let r = self.regexp_exec(re, s)?;
        if self.flags_global(re) || self.flags_sticky(re) {
            self.set_property_object(
                re,
                PropertyKey::from_str("lastIndex"),
                saved,
                &Value::Object(re.clone()),
                true,
            )?;
        }
        if r.is_null() {
            return Ok(Value::Number(-1.0));
        }
        let obj = r.as_object_cloned().unwrap();
        self.get_property(&Value::Object(obj), &PropertyKey::from_str("index"))
    }

    pub fn regexp_symbol_replace(
        &mut self,
        re: &JsObject,
        s: &JsString,
        replace: &Value,
    ) -> JsResult<Value> {
        let global = self.flags_global(re);
        if global {
            self.set_property_object(
                re,
                PropertyKey::from_str("lastIndex"),
                Value::Number(0.0),
                &Value::Object(re.clone()),
                true,
            )?;
        }
        let mut out = JsString::empty();
        let mut last = 0usize;
        loop {
            let r = self.regexp_exec(re, s)?;
            if r.is_null() {
                break;
            }
            let arr = r.as_object_cloned().unwrap();
            let matched =
                self.get_property(&Value::Object(arr.clone()), &PropertyKey::from_str("0"))?;
            let matched = matched.as_string().cloned().unwrap_or_default();
            let index =
                self.get_property(&Value::Object(arr.clone()), &PropertyKey::from_str("index"))?;
            let pos = index.as_number().unwrap_or(0.0) as usize;
            out = out.concat(&s.substring(last, pos));
            let replacement = if self.is_callable_value(replace) {
                let mut call_args = Vec::new();
                call_args.push(Value::String(matched.clone()));
                let len = {
                    let d = arr.borrow();
                    match &d.kind {
                        ObjectKind::Array(a) => a.length as usize,
                        _ => 1,
                    }
                };
                for i in 1..len {
                    call_args.push(self.get_property(
                        &Value::Object(arr.clone()),
                        &PropertyKey::String(JsString::from_str(&i.to_string())),
                    )?);
                }
                call_args.push(Value::Number(pos as f64));
                call_args.push(Value::String(s.clone()));
                let groups = self.get_property(
                    &Value::Object(arr.clone()),
                    &PropertyKey::from_str("groups"),
                )?;
                call_args.push(groups);
                let r = self.call_function(replace, &Value::Undefined, &call_args)?;
                self.to_string(&r)?
            } else {
                let rep = self.to_string(replace)?;
                expand_regexp_replacement(&rep, &arr, s, pos)?
            };
            out = out.concat(&replacement);
            last = pos + matched.len();
            if !global {
                break;
            }
            if matched.is_empty() {
                if last >= s.len() {
                    break;
                }
                // Avoid infinite loops on zero-length matches.
                out = out.concat(&s.substring(last, last + 1));
                last += 1;
            }
        }
        out = out.concat(&s.substring(last, s.len()));
        Ok(Value::String(out))
    }

    pub fn regexp_match_all(&mut self, re: &JsObject, s: &JsString) -> JsResult<Value> {
        // Build an iterator over all match arrays.
        let global = self.flags_global(re);
        if !global {
            return self
                .throw_type_error("String.prototype.matchAll called with a non-global RegExp");
        }
        self.set_property_object(
            re,
            PropertyKey::from_str("lastIndex"),
            Value::Number(0.0),
            &Value::Object(re.clone()),
            true,
        )?;
        let mut results = Vec::new();
        loop {
            let r = self.regexp_exec(re, s)?;
            if r.is_null() {
                break;
            }
            results.push(r);
            // Zero-length guard.
            let last = self.get_property(
                &Value::Object(re.clone()),
                &PropertyKey::from_str("lastIndex"),
            )?;
            if last.as_number().unwrap_or(0.0) as usize > s.len() {
                break;
            }
            if let Some(obj) = results.last().and_then(|v| v.as_object_cloned()) {
                let m = self.get_property(&Value::Object(obj), &PropertyKey::from_str("0"))?;
                if m.as_string().map(|x| x.is_empty()).unwrap_or(false) {
                    let li = last.as_number().unwrap_or(0.0) as usize + 1;
                    self.set_property_object(
                        re,
                        PropertyKey::from_str("lastIndex"),
                        Value::Number(li as f64),
                        &Value::Object(re.clone()),
                        true,
                    )?;
                }
            }
        }
        let arr = self.create_array_from_list(results)?;
        let values = self.get_property(
            &Value::Object(arr.clone()),
            &PropertyKey::from_str("values"),
        )?;
        self.call_function(&values, &Value::Object(arr), &[])
    }

    pub fn regexp_split(&mut self, re: &JsObject, s: &JsString, limit: usize) -> JsResult<Value> {
        let mut parts = Vec::new();
        if limit == 0 {
            return Ok(Value::Object(self.create_array_from_list(parts)?));
        }
        let mut last = 0usize;
        let mut start = 0usize;
        loop {
            match self.regexp_match_at(re, s, start)? {
                Some(m) => {
                    let end = m.end();
                    if end == start && start == last {
                        start += 1;
                        if start > s.len() {
                            break;
                        }
                        continue;
                    }
                    parts.push(Value::String(s.substring(last, m.start())));
                    if parts.len() + 1 >= limit {
                        return Ok(Value::Object(self.create_array_from_list(parts)?));
                    }
                    let groups = m.groups().len();
                    for g in 1..groups {
                        if let Some(r) = m.group(g) {
                            parts.push(Value::String(s.substring(r.start, r.end)));
                        } else {
                            parts.push(Value::Undefined);
                        }
                        if parts.len() + 1 >= limit {
                            return Ok(Value::Object(self.create_array_from_list(parts)?));
                        }
                    }
                    last = end;
                    start = if end == start { end + 1 } else { end };
                    if start > s.len() {
                        break;
                    }
                }
                None => break,
            }
        }
        if last <= s.len() {
            parts.push(Value::String(s.substring(last, s.len())));
        }
        Ok(Value::Object(self.create_array_from_list(parts)?))
    }
}

fn expand_regexp_replacement(
    rep: &JsString,
    arr: &JsObject,
    s: &JsString,
    pos: usize,
) -> JsResult<JsString> {
    let r = rep.to_string_lossy();
    let mut out = String::new();
    let mut chars = r.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                out.push('$');
            }
            Some('&') => {
                chars.next();
                let v = arr.get_hidden("0");
                if let Some(Value::String(v)) = v {
                    out.push_str(&v.to_string_lossy());
                }
            }
            Some('`') => {
                chars.next();
                out.push_str(&s.substring(0, pos).to_string_lossy());
            }
            Some('\'') => {
                chars.next();
                let matched = arr.get_hidden("0");
                let ml = matched
                    .and_then(|v| v.as_string().map(|s| s.len()))
                    .unwrap_or(0);
                out.push_str(&s.substring(pos + ml, s.len()).to_string_lossy());
            }
            Some(d) if d.is_ascii_digit() => {
                chars.next();
                let idx = d.to_digit(10).unwrap() as usize;
                if let Some(Value::String(v)) = arr.get_hidden(&idx.to_string()) {
                    out.push_str(&v.to_string_lossy());
                }
            }
            _ => out.push('$'),
        }
    }
    Ok(JsString::from_str(&out))
}
