//! Array built-in methods.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use std::cell::RefCell;
use std::rc::Rc;

impl Interp {
    pub fn install_array_methods(&mut self) {
        let proto = self.realm.intrinsics.array_prototype.clone();

        self.define_method(&proto, "push", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            for (i, a) in args.iter().enumerate() {
                interp.set_property_object(
                    &obj,
                    PropertyKey::String(JsString::from_str(&(len + i).to_string())),
                    a.clone(),
                    &Value::Object(obj.clone()),
                    true,
                )?;
            }
            let new_len = len + args.len();
            interp.set_property_object(
                &obj,
                PropertyKey::from_str("length"),
                Value::Number(new_len as f64),
                &Value::Object(obj.clone()),
                true,
            )?;
            Ok(Value::Number(new_len as f64))
        });
        self.define_method(&proto, "pop", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            if len == 0 {
                interp.set_property_object(
                    &obj,
                    PropertyKey::from_str("length"),
                    Value::Number(0.0),
                    &Value::Object(obj.clone()),
                    true,
                )?;
                return Ok(Value::Undefined);
            }
            let idx = PropertyKey::String(JsString::from_str(&(len - 1).to_string()));
            let v = interp.get_property(&Value::Object(obj.clone()), &idx)?;
            interp.delete_property(&obj, &idx)?;
            interp.set_property_object(
                &obj,
                PropertyKey::from_str("length"),
                Value::Number((len - 1) as f64),
                &Value::Object(obj.clone()),
                true,
            )?;
            Ok(v)
        });
        self.define_method(&proto, "shift", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            if len == 0 {
                interp.set_property_object(
                    &obj,
                    PropertyKey::from_str("length"),
                    Value::Number(0.0),
                    &Value::Object(obj.clone()),
                    true,
                )?;
                return Ok(Value::Undefined);
            }
            let first = interp.get_property(
                &Value::Object(obj.clone()),
                &PropertyKey::String(JsString::from_str("0")),
            )?;
            for i in 1..len {
                let from = PropertyKey::String(JsString::from_str(&i.to_string()));
                let to = PropertyKey::String(JsString::from_str(&(i - 1).to_string()));
                if interp.has_property(&obj, &from)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &from)?;
                    interp.set_property_object(&obj, to, v, &Value::Object(obj.clone()), true)?;
                } else {
                    interp.delete_property(&obj, &to)?;
                }
            }
            interp.delete_property(
                &obj,
                &PropertyKey::String(JsString::from_str(&(len - 1).to_string())),
            )?;
            interp.set_property_object(
                &obj,
                PropertyKey::from_str("length"),
                Value::Number((len - 1) as f64),
                &Value::Object(obj.clone()),
                true,
            )?;
            Ok(first)
        });
        self.define_method(&proto, "unshift", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let n = args.len();
            for i in (0..len).rev() {
                let from = PropertyKey::String(JsString::from_str(&i.to_string()));
                let to = PropertyKey::String(JsString::from_str(&(i + n).to_string()));
                if interp.has_property(&obj, &from)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &from)?;
                    interp.set_property_object(&obj, to, v, &Value::Object(obj.clone()), true)?;
                } else {
                    interp.delete_property(&obj, &to)?;
                }
            }
            for (i, a) in args.iter().enumerate() {
                interp.set_property_object(
                    &obj,
                    PropertyKey::String(JsString::from_str(&i.to_string())),
                    a.clone(),
                    &Value::Object(obj.clone()),
                    true,
                )?;
            }
            let new_len = len + n;
            interp.set_property_object(
                &obj,
                PropertyKey::from_str("length"),
                Value::Number(new_len as f64),
                &Value::Object(obj.clone()),
                true,
            )?;
            Ok(Value::Number(new_len as f64))
        });
        self.define_method(&proto, "join", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let sep = match args.first() {
                Some(v) if !v.is_undefined() => interp.to_string(v)?,
                _ => JsString::from_str(","),
            };
            let mut out = JsString::empty();
            for i in 0..len {
                if i > 0 {
                    out = out.concat(&sep);
                }
                let v = interp.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                if v.is_nullish() {
                    continue;
                }
                out = out.concat(&interp.to_string(&v)?);
            }
            Ok(Value::String(out))
        });
        self.define_method(&proto, "toString", 0, |interp, this, _args| {
            let join = interp.get_property(this, &PropertyKey::from_str("join"))?;
            if interp.is_callable_value(&join) {
                interp.call_function(&join, this, &[])
            } else {
                let obj = interp.to_object(this)?;
                let len = interp.length_of_array_like(&obj)? as usize;
                let mut out = JsString::empty();
                for i in 0..len {
                    if i > 0 {
                        out = out.concat(&JsString::from_str(","));
                    }
                    let v = interp.get_property(
                        &Value::Object(obj.clone()),
                        &PropertyKey::String(JsString::from_str(&i.to_string())),
                    )?;
                    if !v.is_nullish() {
                        out = out.concat(&interp.to_string(&v)?);
                    }
                }
                Ok(Value::String(out))
            }
        });
        self.define_method(&proto, "toLocaleString", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let mut out = JsString::empty();
            for i in 0..len {
                if i > 0 {
                    out = out.concat(&JsString::from_str(","));
                }
                let v = interp.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                if v.is_nullish() {
                    continue;
                }
                let f = interp.get_property(&v, &PropertyKey::from_str("toLocaleString"))?;
                let s = interp.call_function(&f, &v, &[])?;
                out = out.concat(&interp.to_string(&s)?);
            }
            Ok(Value::String(out))
        });
        self.define_method(&proto, "slice", 2, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let start = relative_index(interp, args.first(), len, 0)?;
            let end = relative_index(interp, args.get(1), len, len)?;
            let count = end.saturating_sub(start);
            let result = interp.new_array_result()?;
            let mut n = 0;
            for i in start..end {
                let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    interp.create_data_property_or_throw(
                        &result,
                        PropertyKey::String(JsString::from_str(&n.to_string())),
                        v,
                    )?;
                }
                n += 1;
            }
            interp.set_array_length_raw(&result, count as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "splice", 2, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let start = relative_index(interp, args.first(), len, 0)?;
            let delete_count = if args.len() < 2 {
                len - start
            } else {
                let dc = interp.to_integer_or_infinity(&args[1])?;
                if dc < 0.0 {
                    0
                } else {
                    (dc as usize).min(len - start)
                }
            };
            let item_count = args.len().saturating_sub(2);
            let result = interp.new_array_result()?;
            for i in 0..delete_count {
                let key = PropertyKey::String(JsString::from_str(&(start + i).to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    interp.create_data_property_or_throw(
                        &result,
                        PropertyKey::String(JsString::from_str(&i.to_string())),
                        v,
                    )?;
                }
            }
            interp.set_array_length_raw(&result, delete_count as u32);
            if item_count < delete_count {
                for i in start..(len - delete_count) {
                    let from =
                        PropertyKey::String(JsString::from_str(&(i + delete_count).to_string()));
                    let to = PropertyKey::String(JsString::from_str(&(i + item_count).to_string()));
                    if interp.has_property(&obj, &from)? {
                        let v = interp.get_property(&Value::Object(obj.clone()), &from)?;
                        interp.set_property_object(
                            &obj,
                            to,
                            v,
                            &Value::Object(obj.clone()),
                            true,
                        )?;
                    } else {
                        interp.delete_property(&obj, &to)?;
                    }
                }
                for i in ((len - delete_count + item_count)..len).rev() {
                    interp.delete_property(
                        &obj,
                        &PropertyKey::String(JsString::from_str(&i.to_string())),
                    )?;
                }
            } else if item_count > delete_count {
                for i in ((start + delete_count)..len).rev() {
                    let from = PropertyKey::String(JsString::from_str(&i.to_string()));
                    let to = PropertyKey::String(JsString::from_str(
                        &(i + item_count - delete_count).to_string(),
                    ));
                    if interp.has_property(&obj, &from)? {
                        let v = interp.get_property(&Value::Object(obj.clone()), &from)?;
                        interp.set_property_object(
                            &obj,
                            to,
                            v,
                            &Value::Object(obj.clone()),
                            true,
                        )?;
                    } else {
                        interp.delete_property(&obj, &to)?;
                    }
                }
            }
            for (i, item) in args.iter().skip(2).enumerate() {
                interp.set_property_object(
                    &obj,
                    PropertyKey::String(JsString::from_str(&(start + i).to_string())),
                    item.clone(),
                    &Value::Object(obj.clone()),
                    true,
                )?;
            }
            let new_len = len - delete_count + item_count;
            interp.set_property_object(
                &obj,
                PropertyKey::from_str("length"),
                Value::Number(new_len as f64),
                &Value::Object(obj.clone()),
                true,
            )?;
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "concat", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let result = interp.new_array_result()?;
            let mut n = 0usize;
            let mut append = |interp: &mut Interp, value: &Value, n: &mut usize| -> JsResult<()> {
                let spreadable = if let Value::Object(o) = value {
                    let spread = interp.get_property(
                        &Value::Object(o.clone()),
                        &PropertyKey::Symbol(
                            interp.realm.intrinsics.symbol_is_concat_spreadable.clone(),
                        ),
                    )?;
                    if spread.is_undefined() {
                        interp.is_array(value)
                    } else {
                        spread.truthy()
                    }
                } else {
                    false
                };
                if spreadable {
                    let len = interp.length_of_array_like(value.as_object().unwrap())? as usize;
                    if (*n as f64) + (len as f64) > 9007199254740991.0 {
                        return interp.throw_type_error("Invalid array length");
                    }
                    for i in 0..len {
                        let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                        if interp.has_property_value(value, &key)? {
                            let v = interp.get_property(value, &key)?;
                            interp.create_data_property_or_throw(
                                &result,
                                PropertyKey::String(JsString::from_str(&n.to_string())),
                                v,
                            )?;
                        }
                        *n += 1;
                    }
                } else {
                    interp.create_data_property_or_throw(
                        &result,
                        PropertyKey::String(JsString::from_str(&n.to_string())),
                        value.clone(),
                    )?;
                    *n += 1;
                }
                Ok(())
            };
            append(interp, &Value::Object(obj), &mut n)?;
            for a in args {
                append(interp, a, &mut n)?;
            }
            interp.set_array_length_raw(&result, n as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "indexOf", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            let mut k = match args.get(1) {
                Some(v) if !v.is_undefined() => {
                    let n = interp.to_integer_or_infinity(v)?;
                    if n >= len as f64 {
                        return Ok(Value::Number(-1.0));
                    }
                    if n >= 0.0 {
                        n as usize
                    } else {
                        ((len as f64 + n).max(0.0)) as usize
                    }
                }
                _ => 0,
            };
            while k < len {
                let key = PropertyKey::String(JsString::from_str(&k.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    if Value::strict_equals(&v, &target) {
                        return Ok(Value::Number(k as f64));
                    }
                }
                k += 1;
            }
            Ok(Value::Number(-1.0))
        });
        self.define_method(&proto, "lastIndexOf", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            if len == 0 {
                return Ok(Value::Number(-1.0));
            }
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            let mut k = match args.get(1) {
                Some(v) if !v.is_undefined() => {
                    let n = interp.to_integer_or_infinity(v)?;
                    if n < 0.0 {
                        (len as f64 + n) as isize
                    } else {
                        n.min(len as f64 - 1.0) as isize
                    }
                }
                _ => len as isize - 1,
            };
            while k >= 0 {
                let key = PropertyKey::String(JsString::from_str(&(k as usize).to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    if Value::strict_equals(&v, &target) {
                        return Ok(Value::Number(k as f64));
                    }
                }
                k -= 1;
            }
            Ok(Value::Number(-1.0))
        });
        self.define_method(&proto, "includes", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            let mut k = match args.get(1) {
                Some(v) if !v.is_undefined() => {
                    let n = interp.to_integer_or_infinity(v)?;
                    if n >= len as f64 {
                        return Ok(Value::Bool(false));
                    }
                    if n >= 0.0 {
                        n as usize
                    } else {
                        ((len as f64 + n).max(0.0)) as usize
                    }
                }
                _ => 0,
            };
            while k < len {
                let v = interp.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&k.to_string())),
                )?;
                if Value::same_value_zero(&v, &target) {
                    return Ok(Value::Bool(true));
                }
                k += 1;
            }
            Ok(Value::Bool(false))
        });
        self.define_method(&proto, "forEach", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            if !interp.is_callable_value(&cb) {
                return interp.throw_type_error("callback is not a function");
            }
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            for i in 0..len {
                let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    interp.call_function(
                        &cb,
                        &this_arg,
                        &[v, Value::Number(i as f64), Value::Object(obj.clone())],
                    )?;
                }
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "map", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            if len as f64 > 4294967295.0 {
                return interp.throw_range_error("Invalid array length");
            }
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            if !interp.is_callable_value(&cb) {
                return interp.throw_type_error("callback is not a function");
            }
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let result = interp.new_array_result()?;
            for i in 0..len {
                let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    let mapped = interp.call_function(
                        &cb,
                        &this_arg,
                        &[v, Value::Number(i as f64), Value::Object(obj.clone())],
                    )?;
                    interp.create_data_property_or_throw(
                        &result,
                        PropertyKey::String(JsString::from_str(&i.to_string())),
                        mapped,
                    )?;
                }
            }
            interp.set_array_length_raw(&result, len as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "filter", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            if !interp.is_callable_value(&cb) {
                return interp.throw_type_error("callback is not a function");
            }
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let result = interp.new_array_result()?;
            let mut n = 0;
            for i in 0..len {
                let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    let keep = interp.call_function(
                        &cb,
                        &this_arg,
                        &[
                            v.clone(),
                            Value::Number(i as f64),
                            Value::Object(obj.clone()),
                        ],
                    )?;
                    if keep.truthy() {
                        interp.create_data_property_or_throw(
                            &result,
                            PropertyKey::String(JsString::from_str(&n.to_string())),
                            v,
                        )?;
                        n += 1;
                    }
                }
            }
            interp.set_array_length_raw(&result, n);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "every", 1, |interp, this, args| {
            array_predicate_all(interp, this, args, true)
        });
        self.define_method(&proto, "some", 1, |interp, this, args| {
            array_predicate_all(interp, this, args, false)
        });
        self.define_method(&proto, "find", 1, |interp, this, args| {
            array_find(interp, this, args, FindMode::Value)
        });
        self.define_method(&proto, "findIndex", 1, |interp, this, args| {
            array_find(interp, this, args, FindMode::Index)
        });
        self.define_method(&proto, "findLast", 1, |interp, this, args| {
            array_find(interp, this, args, FindMode::LastValue)
        });
        self.define_method(&proto, "findLastIndex", 1, |interp, this, args| {
            array_find(interp, this, args, FindMode::LastIndex)
        });
        self.define_method(&proto, "reduce", 1, |interp, this, args| {
            array_reduce(interp, this, args, false)
        });
        self.define_method(&proto, "reduceRight", 1, |interp, this, args| {
            array_reduce(interp, this, args, true)
        });
        self.define_method(&proto, "fill", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let start = relative_index(interp, args.get(1), len, 0)?;
            let end = relative_index(interp, args.get(2), len, len)?;
            for i in start..end {
                interp.set_property_object(
                    &obj,
                    PropertyKey::String(JsString::from_str(&i.to_string())),
                    value.clone(),
                    &Value::Object(obj.clone()),
                    true,
                )?;
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&proto, "copyWithin", 2, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let target = relative_index(interp, args.first(), len, 0)?;
            let start = relative_index(interp, args.get(1), len, 0)?;
            let end = relative_index(interp, args.get(2), len, len)?;
            let count = (end - start).min(len - target);
            let mut values = Vec::new();
            for i in 0..count {
                let key = PropertyKey::String(JsString::from_str(&(start + i).to_string()));
                let has = interp.has_property(&obj, &key)?;
                values.push(if has {
                    Some(interp.get_property(&Value::Object(obj.clone()), &key)?)
                } else {
                    None
                });
            }
            for (i, v) in values.into_iter().enumerate() {
                let key = PropertyKey::String(JsString::from_str(&(target + i).to_string()));
                match v {
                    Some(v) => {
                        interp.set_property_object(
                            &obj,
                            key,
                            v,
                            &Value::Object(obj.clone()),
                            true,
                        )?;
                    }
                    None => {
                        interp.delete_property(&obj, &key)?;
                    }
                }
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&proto, "reverse", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            for lower in 0..(len / 2) {
                let upper = len - lower - 1;
                let lk = PropertyKey::String(JsString::from_str(&lower.to_string()));
                let uk = PropertyKey::String(JsString::from_str(&upper.to_string()));
                let lower_exists = interp.has_property(&obj, &lk)?;
                let upper_exists = interp.has_property(&obj, &uk)?;
                let lower_v = if lower_exists {
                    Some(interp.get_property(&Value::Object(obj.clone()), &lk)?)
                } else {
                    None
                };
                let upper_v = if upper_exists {
                    Some(interp.get_property(&Value::Object(obj.clone()), &uk)?)
                } else {
                    None
                };
                match upper_v {
                    Some(v) => {
                        interp.set_property_object(
                            &obj,
                            lk,
                            v,
                            &Value::Object(obj.clone()),
                            true,
                        )?;
                    }
                    None => {
                        interp.delete_property(&obj, &lk)?;
                    }
                }
                match lower_v {
                    Some(v) => {
                        interp.set_property_object(
                            &obj,
                            uk,
                            v,
                            &Value::Object(obj.clone()),
                            true,
                        )?;
                    }
                    None => {
                        interp.delete_property(&obj, &uk)?;
                    }
                }
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&proto, "sort", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let compare = args.first().cloned().unwrap_or(Value::Undefined);
            let has_compare = !compare.is_undefined();
            if has_compare && !interp.is_callable_value(&compare) {
                return interp.throw_type_error("comparator is not a function");
            }
            // Collect present values.
            let mut values: Vec<Value> = Vec::new();
            let mut undefined_count = 0usize;
            for i in 0..len {
                let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    if v.is_undefined() {
                        undefined_count += 1;
                    } else {
                        values.push(v);
                    }
                }
            }
            if has_compare {
                let mut err: Option<Completion> = None;
                values.sort_by(|a, b| {
                    if err.is_some() {
                        return std::cmp::Ordering::Equal;
                    }
                    match interp.call_function(&compare, &Value::Undefined, &[a.clone(), b.clone()])
                    {
                        Ok(v) => {
                            let n = interp.to_number(&v).unwrap_or(f64::NAN);
                            if n < 0.0 {
                                std::cmp::Ordering::Less
                            } else if n > 0.0 {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Equal
                            }
                        }
                        Err(c) => {
                            err = Some(c);
                            std::cmp::Ordering::Equal
                        }
                    }
                });
                if let Some(c) = err {
                    return Err(c);
                }
            } else {
                let mut pairs: Vec<(JsString, Value)> = Vec::with_capacity(values.len());
                for v in values.drain(..) {
                    pairs.push((interp.to_string(&v)?, v));
                }
                pairs.sort_by(|a, b| a.0.as_slice().cmp(b.0.as_slice()));
                values = pairs.into_iter().map(|(_, v)| v).collect();
            }
            let mut i = 0;
            for v in values {
                interp.set_property_object(
                    &obj,
                    PropertyKey::String(JsString::from_str(&i.to_string())),
                    v,
                    &Value::Object(obj.clone()),
                    true,
                )?;
                i += 1;
            }
            for _ in 0..undefined_count {
                interp.set_property_object(
                    &obj,
                    PropertyKey::String(JsString::from_str(&i.to_string())),
                    Value::Undefined,
                    &Value::Object(obj.clone()),
                    true,
                )?;
                i += 1;
            }
            while i < len {
                interp.delete_property(
                    &obj,
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                i += 1;
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&proto, "at", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let n = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            let idx = if n < 0.0 { len as f64 + n } else { n };
            if idx < 0.0 || idx >= len as f64 {
                return Ok(Value::Undefined);
            }
            interp.get_property(
                &Value::Object(obj),
                &PropertyKey::String(JsString::from_str(&(idx as usize).to_string())),
            )
        });
        self.define_method(&proto, "keys", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            make_array_iterator(interp, obj, len, ArrayIteratorKind::Key)
        });
        self.define_method(&proto, "values", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            make_array_iterator(interp, obj, len, ArrayIteratorKind::Value)
        });
        self.define_method(&proto, "entries", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            make_array_iterator(interp, obj, len, ArrayIteratorKind::Entry)
        });
        let values_fn = self
            .get_property(
                &Value::Object(proto.clone()),
                &PropertyKey::from_str("values"),
            )
            .ok();
        if let Some(v) = values_fn {
            let desc = Descriptor {
                value: Some(v),
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
        self.define_method(&proto, "flat", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let depth = match args.first() {
                Some(v) if !v.is_undefined() => interp.to_integer_or_infinity(v)?.max(0.0) as usize,
                _ => 1,
            };
            let result = interp.new_array_result()?;
            let mut n = 0usize;
            flatten_into(interp, &obj, depth, &result, &mut n)?;
            interp.set_array_length_raw(&result, n as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "flatMap", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            if !interp.is_callable_value(&cb) {
                return interp.throw_type_error("callback is not a function");
            }
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let result = interp.new_array_result()?;
            let mut n = 0usize;
            for i in 0..len {
                let key = PropertyKey::String(JsString::from_str(&i.to_string()));
                if interp.has_property(&obj, &key)? {
                    let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
                    let mapped = interp.call_function(
                        &cb,
                        &this_arg,
                        &[v, Value::Number(i as f64), Value::Object(obj.clone())],
                    )?;
                    flatten_one(interp, &mapped, &result, &mut n)?;
                }
            }
            interp.set_array_length_raw(&result, n as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "with", 2, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let rel = interp.to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?;
            let idx = if rel < 0.0 { len as f64 + rel } else { rel };
            if idx < 0.0 || idx >= len as f64 {
                return interp.throw_range_error("Invalid index");
            }
            let idx = idx as usize;
            let result = interp.new_array_result()?;
            for i in 0..len {
                let v = if i == idx {
                    args.get(1).cloned().unwrap_or(Value::Undefined)
                } else {
                    interp.get_property(
                        &Value::Object(obj.clone()),
                        &PropertyKey::String(JsString::from_str(&i.to_string())),
                    )?
                };
                interp.create_data_property_or_throw(
                    &result,
                    PropertyKey::String(JsString::from_str(&i.to_string())),
                    v,
                )?;
            }
            interp.set_array_length_raw(&result, len as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "toReversed", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let result = interp.new_array_result()?;
            for i in 0..len {
                let v = interp.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&(len - 1 - i).to_string())),
                )?;
                interp.create_data_property_or_throw(
                    &result,
                    PropertyKey::String(JsString::from_str(&i.to_string())),
                    v,
                )?;
            }
            interp.set_array_length_raw(&result, len as u32);
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "toSorted", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let list = interp.iterable_to_list(&Value::Object(obj))?;
            let arr = interp.create_array_from_list(list)?;
            let sort =
                interp.get_property(&Value::Object(arr.clone()), &PropertyKey::from_str("sort"))?;
            interp.call_function(&sort, &Value::Object(arr.clone()), args)?;
            Ok(Value::Object(arr))
        });
        self.define_method(&proto, "toSpliced", 2, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let list = interp.iterable_to_list(&Value::Object(obj))?;
            let copy = interp.create_array_from_list(list)?;
            let splice = interp.get_property(
                &Value::Object(copy.clone()),
                &PropertyKey::from_str("splice"),
            )?;
            interp.call_function(&splice, &Value::Object(copy.clone()), args)?;
            Ok(Value::Object(copy))
        });

        // Array.from / Array.of
        let ctor = self.realm.intrinsics.array_ctor.clone().unwrap();
        self.define_method(&ctor, "from", 1, |interp, _this, args| {
            let items = args.first().cloned().unwrap_or(Value::Undefined);
            let map_fn = args.get(1).cloned().unwrap_or(Value::Undefined);
            let this_arg = args.get(2).cloned().unwrap_or(Value::Undefined);
            let mut values = Vec::new();
            if !items.is_nullish() {
                let iter_method = interp.get_method(
                    &items,
                    &PropertyKey::Symbol(interp.realm.intrinsics.symbol_iterator.clone()),
                )?;
                if let Some(m) = iter_method {
                    let iter = interp.call_function(&m, &items, &[])?;
                    let next = interp.get_property(&iter, &PropertyKey::from_str("next"))?;
                    loop {
                        let r = interp.call_function(&next, &iter, &[])?;
                        let r_obj =
                            r.as_object_cloned()
                                .ok_or(Completion::Normal)
                                .or_else(|_| {
                                    interp.throw_type_error("Iterator result is not an object")
                                })?;
                        let done =
                            interp.get_property_key(&Value::Object(r_obj.clone()), "done")?;
                        if done.truthy() {
                            break;
                        }
                        values.push(interp.get_property_key(&Value::Object(r_obj), "value")?);
                    }
                } else {
                    let obj = interp.to_object(&items)?;
                    let len = interp.length_of_array_like(&obj)? as usize;
                    for i in 0..len {
                        values.push(interp.get_property(
                            &Value::Object(obj.clone()),
                            &PropertyKey::String(JsString::from_str(&i.to_string())),
                        )?);
                    }
                }
            }
            if interp.is_callable_value(&map_fn) {
                let mapped: Result<Vec<Value>, Completion> = values
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| {
                        interp.call_function(&map_fn, &this_arg, &[v, Value::Number(i as f64)])
                    })
                    .collect();
                values = mapped?;
            }
            let arr = interp.create_array_from_list(values)?;
            Ok(Value::Object(arr))
        });
        self.define_method(&ctor, "of", 0, |interp, _this, args| {
            let arr = interp.create_array_from_list(args.to_vec())?;
            Ok(Value::Object(arr))
        });
    }

    pub fn new_array_result(&mut self) -> JsResult<JsObject> {
        Ok(JsObject::array(Some(
            self.realm.intrinsics.array_prototype.clone(),
        )))
    }

    pub fn set_array_length_raw(&mut self, arr: &JsObject, len: u32) {
        if let ObjectKind::Array(a) = &mut arr.0.borrow_mut().kind {
            a.length = len;
        }
    }
}

fn relative_index(
    interp: &mut Interp,
    value: Option<&Value>,
    len: usize,
    default: usize,
) -> JsResult<usize> {
    match value {
        None => Ok(default),
        Some(v) if v.is_undefined() => Ok(default),
        Some(v) => {
            let n = interp.to_integer_or_infinity(v)?;
            if n < 0.0 {
                Ok(((len as f64 + n).max(0.0)) as usize)
            } else {
                Ok((n as usize).min(len))
            }
        }
    }
}

fn array_predicate_all(
    interp: &mut Interp,
    this: &Value,
    args: &[Value],
    every: bool,
) -> JsResult<Value> {
    let obj = interp.to_object(this)?;
    let len = interp.length_of_array_like(&obj)? as usize;
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
    for i in 0..len {
        let key = PropertyKey::String(JsString::from_str(&i.to_string()));
        if interp.has_property(&obj, &key)? {
            let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
            let r = interp.call_function(
                &cb,
                &this_arg,
                &[v, Value::Number(i as f64), Value::Object(obj.clone())],
            )?;
            if every && !r.truthy() {
                return Ok(Value::Bool(false));
            }
            if !every && r.truthy() {
                return Ok(Value::Bool(true));
            }
        }
    }
    Ok(Value::Bool(every))
}

enum FindMode {
    Value,
    Index,
    LastValue,
    LastIndex,
}

fn array_find(
    interp: &mut Interp,
    this: &Value,
    args: &[Value],
    mode: FindMode,
) -> JsResult<Value> {
    let obj = interp.to_object(this)?;
    let len = interp.length_of_array_like(&obj)? as usize;
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
    let rev = matches!(mode, FindMode::LastValue | FindMode::LastIndex);
    if len == 0 {
        return Ok(match mode {
            FindMode::Value | FindMode::LastValue => Value::Undefined,
            FindMode::Index | FindMode::LastIndex => Value::Number(-1.0),
        });
    }
    let mut i: i128 = if rev { len as i128 - 1 } else { 0 };
    let step: i128 = if rev { -1 } else { 1 };
    loop {
        if i < 0 || i >= len as i128 {
            break;
        }
        let idx = i as usize;
        let v = interp.get_property(
            &Value::Object(obj.clone()),
            &PropertyKey::String(JsString::from_str(&idx.to_string())),
        )?;
        let r = interp.call_function(
            &cb,
            &this_arg,
            &[
                v.clone(),
                Value::Number(idx as f64),
                Value::Object(obj.clone()),
            ],
        )?;
        if r.truthy() {
            return Ok(match mode {
                FindMode::Value | FindMode::LastValue => v,
                FindMode::Index | FindMode::LastIndex => Value::Number(idx as f64),
            });
        }
        i += step;
    }
    Ok(match mode {
        FindMode::Value | FindMode::LastValue => Value::Undefined,
        FindMode::Index | FindMode::LastIndex => Value::Number(-1.0),
    })
}

fn array_reduce(interp: &mut Interp, this: &Value, args: &[Value], right: bool) -> JsResult<Value> {
    let obj = interp.to_object(this)?;
    let len = interp.length_of_array_like(&obj)? as usize;
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    if !interp.is_callable_value(&cb) {
        return interp.throw_type_error("callback is not a function");
    }
    let mut acc = args.get(1).cloned();
    if len == 0 {
        return match acc {
            Some(a) => Ok(a),
            None => interp.throw_type_error("Reduce of empty array with no initial value"),
        };
    }
    let mut i: i128 = if right { len as i128 - 1 } else { 0 };
    let step: i128 = if right { -1 } else { 1 };
    loop {
        if i < 0 || i >= len as i128 {
            break;
        }
        let idx = i as usize;
        let key = PropertyKey::String(JsString::from_str(&idx.to_string()));
        if interp.has_property(&obj, &key)? {
            let v = interp.get_property(&Value::Object(obj.clone()), &key)?;
            acc = Some(match acc {
                None => v,
                Some(a) => interp.call_function(
                    &cb,
                    &Value::Undefined,
                    &[a, v, Value::Number(idx as f64), Value::Object(obj.clone())],
                )?,
            });
        }
        i += step;
    }
    match acc {
        Some(a) => Ok(a),
        None => interp.throw_type_error("Reduce of empty array with no initial value"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArrayIteratorKind {
    Key,
    Value,
    Entry,
}

fn make_array_iterator(
    interp: &mut Interp,
    target: JsObject,
    len: usize,
    kind: ArrayIteratorKind,
) -> JsResult<Value> {
    let index = Rc::new(RefCell::new(0usize));
    let next = native_fn("next", 0, move |interp, _this, _args| {
        let mut i = index.borrow_mut();
        if *i >= len {
            let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
            obj.put_hidden("value", Value::Undefined);
            obj.put_hidden("done", Value::Bool(true));
            return Ok(Value::Object(obj));
        }
        let idx = *i;
        *i += 1;
        let value = match kind {
            ArrayIteratorKind::Key => Value::Number(idx as f64),
            ArrayIteratorKind::Value => interp.get_property(
                &Value::Object(target.clone()),
                &PropertyKey::String(JsString::from_str(&idx.to_string())),
            )?,
            ArrayIteratorKind::Entry => {
                let v = interp.get_property(
                    &Value::Object(target.clone()),
                    &PropertyKey::String(JsString::from_str(&idx.to_string())),
                )?;
                Value::Object(interp.create_array_from_list(vec![Value::Number(idx as f64), v])?)
            }
        };
        let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
        obj.put_hidden("value", value);
        obj.put_hidden("done", Value::Bool(false));
        Ok(Value::Object(obj))
    });
    next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
    let iter = JsObject::ordinary(Some(
        interp.realm.intrinsics.array_iterator_prototype.clone(),
    ));
    iter.put_hidden("next", Value::Object(next));
    Ok(Value::Object(iter))
}

fn flatten_one(
    interp: &mut Interp,
    value: &Value,
    target: &JsObject,
    n: &mut usize,
) -> JsResult<()> {
    if interp.is_array(value) {
        let len = interp.length_of_array_like(value.as_object().unwrap())? as usize;
        for i in 0..len {
            let key = PropertyKey::String(JsString::from_str(&i.to_string()));
            if interp.has_property_value(value, &key)? {
                let v = interp.get_property(value, &key)?;
                interp.create_data_property_or_throw(
                    target,
                    PropertyKey::String(JsString::from_str(&n.to_string())),
                    v,
                )?;
                *n += 1;
            }
        }
    } else {
        interp.create_data_property_or_throw(
            target,
            PropertyKey::String(JsString::from_str(&n.to_string())),
            value.clone(),
        )?;
        *n += 1;
    }
    Ok(())
}

fn flatten_into(
    interp: &mut Interp,
    source: &JsObject,
    depth: usize,
    target: &JsObject,
    n: &mut usize,
) -> JsResult<()> {
    let len = interp.length_of_array_like(source)? as usize;
    for i in 0..len {
        let key = PropertyKey::String(JsString::from_str(&i.to_string()));
        if !interp.has_property(source, &key)? {
            continue;
        }
        let v = interp.get_property(&Value::Object(source.clone()), &key)?;
        if depth > 0 && interp.is_array(&v) {
            flatten_into(interp, v.as_object().unwrap(), depth - 1, target, n)?;
        } else {
            interp.create_data_property_or_throw(
                target,
                PropertyKey::String(JsString::from_str(&n.to_string())),
                v,
            )?;
            *n += 1;
        }
    }
    Ok(())
}
