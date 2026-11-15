//! Iterator helpers and the Iterator constructor.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

impl Interp {
    pub fn install_iterator_helpers(&mut self) {
        let proto = self.realm.intrinsics.iterator_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "Iterator",
            0,
            |interp, _this, _args| {
                interp.throw_type_error("Iterator is not directly constructible")
            },
            |interp, _args, _nt| interp.throw_type_error("Iterator is not directly constructible"),
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("Iterator", Value::Object(ctor.clone()));
        self.define_method(&ctor, "from", 1, |interp, _this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            if let Some(o) = v.as_object_cloned() {
                let next = interp
                    .get_property(&Value::Object(o.clone()), &PropertyKey::from_str("next"))?;
                if interp.is_callable_value(&next) {
                    return Ok(v);
                }
            }
            let iter = interp.get_iterator(&v)?;
            let record = iter.as_object_cloned().unwrap();
            let record2 = record.clone();
            let next = native_fn("next", 0, move |interp, _this, _args| {
                let (done, value) = interp.iterator_next(&Value::Object(record2.clone()))?;
                iterator_result(interp, value, done)
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let wrapper =
                JsObject::ordinary(Some(interp.realm.intrinsics.iterator_prototype.clone()));
            wrapper.put_hidden("next", Value::Object(next));
            Ok(Value::Object(wrapper))
        });
        let tag = Descriptor {
            value: Some(Value::String(JsString::from_str("Iterator"))),
            writable: Some(false),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(
            &proto,
            PropertyKey::Symbol(self.realm.intrinsics.symbol_to_string_tag.clone()),
            tag,
        );

        self.define_method(&proto, "map", 1, |interp, this, args| {
            let mapper = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            let next = native_fn("next", 0, move |interp, _this, _args| {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    return iterator_result(interp, Value::Undefined, true);
                }
                let mapped = interp.call_function(&mapper, &Value::Undefined, &[value])?;
                iterator_result(interp, mapped, false)
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let helper =
                JsObject::ordinary(Some(interp.realm.intrinsics.iterator_prototype.clone()));
            helper.put_hidden("next", Value::Object(next));
            Ok(Value::Object(helper))
        });
        self.define_method(&proto, "filter", 1, |interp, this, args| {
            let pred = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            let next = native_fn("next", 0, move |interp, _this, _args| loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    return iterator_result(interp, Value::Undefined, true);
                }
                let keep = interp.call_function(&pred, &Value::Undefined, &[value.clone()])?;
                if keep.truthy() {
                    return iterator_result(interp, value, false);
                }
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let helper =
                JsObject::ordinary(Some(interp.realm.intrinsics.iterator_prototype.clone()));
            helper.put_hidden("next", Value::Object(next));
            Ok(Value::Object(helper))
        });
        self.define_method(&proto, "take", 1, |interp, this, args| {
            let remaining = Rc::new(RefCell::new(
                interp
                    .to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?
                    .max(0.0) as usize,
            ));
            let source = interp.to_object(this)?;
            let next = native_fn("next", 0, move |interp, _this, _args| {
                if *remaining.borrow() == 0 {
                    return iterator_result(interp, Value::Undefined, true);
                }
                *remaining.borrow_mut() -= 1;
                iterator_next(interp, &source)
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let helper =
                JsObject::ordinary(Some(interp.realm.intrinsics.iterator_prototype.clone()));
            helper.put_hidden("next", Value::Object(next));
            Ok(Value::Object(helper))
        });
        self.define_method(&proto, "drop", 1, |interp, this, args| {
            let remaining = Rc::new(RefCell::new(
                interp
                    .to_integer_or_infinity(args.first().unwrap_or(&Value::Undefined))?
                    .max(0.0) as usize,
            ));
            let source = interp.to_object(this)?;
            let next = native_fn("next", 0, move |interp, _this, _args| {
                while *remaining.borrow() > 0 {
                    *remaining.borrow_mut() -= 1;
                    let r = iterator_next(interp, &source)?;
                    let (_, done) = iterator_result_parts(interp, r)?;
                    if done {
                        return iterator_result(interp, Value::Undefined, true);
                    }
                }
                iterator_next(interp, &source)
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let helper =
                JsObject::ordinary(Some(interp.realm.intrinsics.iterator_prototype.clone()));
            helper.put_hidden("next", Value::Object(next));
            Ok(Value::Object(helper))
        });
        self.define_method(&proto, "flatMap", 1, |interp, this, args| {
            let mapper = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            let inner: Rc<RefCell<Option<JsObject>>> = Rc::new(RefCell::new(None));
            let next = native_fn("next", 0, move |interp, _this, _args| loop {
                if let Some(inner_iter) = inner.borrow().clone() {
                    let r = iterator_next(interp, &inner_iter)?;
                    let (value, done) = iterator_result_parts(interp, r)?;
                    if !done {
                        return iterator_result(interp, value, false);
                    }
                    *inner.borrow_mut() = None;
                }
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    return iterator_result(interp, Value::Undefined, true);
                }
                let mapped = interp.call_function(&mapper, &Value::Undefined, &[value])?;
                let iter = interp.get_iterator(&mapped)?;
                *inner.borrow_mut() = iter.as_object_cloned();
            });
            next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let helper =
                JsObject::ordinary(Some(interp.realm.intrinsics.iterator_prototype.clone()));
            helper.put_hidden("next", Value::Object(next));
            Ok(Value::Object(helper))
        });
        self.define_method(&proto, "forEach", 1, |interp, this, args| {
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    break;
                }
                interp.call_function(&cb, &Value::Undefined, &[value])?;
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "toArray", 0, |interp, this, _args| {
            let source = interp.to_object(this)?;
            let mut vals = Vec::new();
            loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    break;
                }
                vals.push(value);
            }
            Ok(Value::Object(interp.create_array_from_list(vals)?))
        });
        self.define_method(&proto, "reduce", 1, |interp, this, args| {
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            let mut acc = args.get(1).cloned();
            let source = interp.to_object(this)?;
            loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    break;
                }
                acc = Some(match acc {
                    None => value,
                    Some(a) => interp.call_function(&cb, &Value::Undefined, &[a, value])?,
                });
            }
            match acc {
                Some(a) => Ok(a),
                None => interp.throw_type_error("Reduce of empty iterator with no initial value"),
            }
        });
        self.define_method(&proto, "some", 1, |interp, this, args| {
            let pred = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    return Ok(Value::Bool(false));
                }
                let keep = interp.call_function(&pred, &Value::Undefined, &[value])?;
                if keep.truthy() {
                    return Ok(Value::Bool(true));
                }
            }
        });
        self.define_method(&proto, "every", 1, |interp, this, args| {
            let pred = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    return Ok(Value::Bool(true));
                }
                let keep = interp.call_function(&pred, &Value::Undefined, &[value])?;
                if !keep.truthy() {
                    return Ok(Value::Bool(false));
                }
            }
        });
        self.define_method(&proto, "find", 1, |interp, this, args| {
            let pred = args.first().cloned().unwrap_or(Value::Undefined);
            let source = interp.to_object(this)?;
            loop {
                let r = iterator_next(interp, &source)?;
                let (value, done) = iterator_result_parts(interp, r)?;
                if done {
                    return Ok(Value::Undefined);
                }
                let keep = interp.call_function(&pred, &Value::Undefined, &[value.clone()])?;
                if keep.truthy() {
                    return Ok(value);
                }
            }
        });
    }
}

fn iterator_next(interp: &mut Interp, source: &JsObject) -> JsResult<Value> {
    let next = interp.get_property(
        &Value::Object(source.clone()),
        &PropertyKey::from_str("next"),
    )?;
    interp.call_function(&next, &Value::Object(source.clone()), &[])
}

fn iterator_result_parts(interp: &mut Interp, r: Value) -> JsResult<(Value, bool)> {
    let obj = match r.as_object_cloned() {
        Some(o) => o,
        None => return interp.throw_type_error("Iterator result is not an object"),
    };
    let done = interp.get_property(&Value::Object(obj.clone()), &PropertyKey::from_str("done"))?;
    let value = interp.get_property(&Value::Object(obj), &PropertyKey::from_str("value"))?;
    Ok((value, done.truthy()))
}

fn iterator_result(interp: &mut Interp, value: Value, done: bool) -> JsResult<Value> {
    let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
    obj.put_hidden("value", value);
    obj.put_hidden("done", Value::Bool(done));
    Ok(Value::Object(obj))
}
