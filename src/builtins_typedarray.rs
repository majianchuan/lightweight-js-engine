//! ArrayBuffer, TypedArray and DataView built-ins.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

impl Interp {
    pub fn install_typed_arrays(&mut self) {
        self.install_array_buffer();
        self.install_typed_array_base();
        self.install_typed_array_ctors();
        self.install_data_view();
    }

    fn install_array_buffer(&mut self) {
        let proto = self.realm.intrinsics.array_buffer_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "ArrayBuffer",
            1,
            |interp, _this, _args| {
                interp.throw_type_error("Constructor ArrayBuffer requires 'new'")
            },
            |interp, args, new_target| {
                let len = match args.first() {
                    Some(v) => interp.to_index(v)?,
                    None => 0,
                };
                let p = interp.prototype_from_new_target(new_target);
                let bytes = vec![0u8; len];
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::ArrayBuffer(ArrayBufferData {
                        bytes: Rc::new(RefCell::new(bytes)),
                        detached: false,
                        max_byte_length: None,
                        resizable: false,
                    }),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("ArrayBuffer", Value::Object(ctor.clone()));
        self.realm.intrinsics.array_buffer_ctor = Some(ctor.clone());
        self.define_method(&ctor, "isView", 1, |interp, _this, args| {
            let is_view = args
                .first()
                .and_then(|v| v.as_object_cloned())
                .map(|o| {
                    matches!(
                        &o.borrow().kind,
                        ObjectKind::TypedArray(_) | ObjectKind::DataView(_)
                    )
                })
                .unwrap_or(false);
            let _ = interp;
            Ok(Value::Bool(is_view))
        });
        let byte_length = self.native_method("get byteLength", 0, |interp, this, _args| {
            let o = interp.array_buffer_object(this)?;
            let n = interp.array_buffer_len(&o);
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&proto, "byteLength", byte_length.clone(), None);
        self.define_accessor_public(&proto, "maxByteLength", byte_length, None);
        self.define_method(&proto, "slice", 2, |interp, this, args| {
            let o = interp.array_buffer_object(this)?;
            let len = interp.array_buffer_len(&o);
            let start = clamp_rel(interp, args.first(), len, 0)?;
            let end = clamp_rel(interp, args.get(1), len, len)?;
            let bytes = interp.array_buffer_bytes(&o);
            let slice = bytes[start.min(end)..end].to_vec();
            let new = JsObject::new(ObjectData {
                prototype: Some(interp.realm.intrinsics.array_buffer_prototype.clone()),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: Rc::new(RefCell::new(slice)),
                    detached: false,
                    max_byte_length: None,
                    resizable: false,
                }),
                private: IndexMap::new(),
            });
            Ok(Value::Object(new))
        });
        let tag = Descriptor {
            value: Some(Value::String(JsString::from_str("ArrayBuffer"))),
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

        // SharedArrayBuffer (same representation, non-detachable).
        let shared_proto = self.realm.intrinsics.shared_array_buffer_prototype.clone();
        let shared = crate::builtins::native_ctor(
            "SharedArrayBuffer",
            1,
            |interp, _this, _args| {
                interp.throw_type_error("Constructor SharedArrayBuffer requires 'new'")
            },
            |interp, args, new_target| {
                let len = match args.first() {
                    Some(v) => interp.to_index(v)?,
                    None => 0,
                };
                let p = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::SharedArrayBuffer(ArrayBufferData {
                        bytes: Rc::new(RefCell::new(vec![0u8; len])),
                        detached: false,
                        max_byte_length: None,
                        resizable: false,
                    }),
                    private: IndexMap::new(),
                })))
            },
        );
        shared.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        shared.define_fixed("prototype", Value::Object(shared_proto.clone()));
        shared_proto.put_hidden("constructor", Value::Object(shared.clone()));
        self.realm
            .global
            .put_hidden("SharedArrayBuffer", Value::Object(shared.clone()));
        self.realm.intrinsics.shared_array_buffer_ctor = Some(shared);
        let bl = self.native_method("get byteLength", 0, |interp, this, _args| {
            let o = interp.array_buffer_object(this)?;
            let n = interp.array_buffer_len(&o);
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&shared_proto, "byteLength", bl.clone(), None);
        self.define_accessor_public(&shared_proto, "maxByteLength", bl, None);
        self.define_method(&shared_proto, "slice", 2, |interp, this, args| {
            let o = interp.array_buffer_object(this)?;
            let len = interp.array_buffer_len(&o);
            let start = clamp_rel(interp, args.first(), len, 0)?;
            let end = clamp_rel(interp, args.get(1), len, len)?;
            let bytes = interp.array_buffer_bytes(&o);
            let slice = bytes[start.min(end)..end].to_vec();
            let new = JsObject::new(ObjectData {
                prototype: Some(
                    interp
                        .realm
                        .intrinsics
                        .shared_array_buffer_prototype
                        .clone(),
                ),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::SharedArrayBuffer(ArrayBufferData {
                    bytes: Rc::new(RefCell::new(slice)),
                    detached: false,
                    max_byte_length: None,
                    resizable: false,
                }),
                private: IndexMap::new(),
            });
            Ok(Value::Object(new))
        });
    }

    fn install_typed_array_base(&mut self) {
        let proto = self.realm.intrinsics.typed_array_prototype.clone();
        let len_getter = self.native_method("get length", 0, |interp, this, _args| {
            let ta = interp.typed_array_object(this)?;
            let n = interp.typed_array_len(&ta);
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&proto, "length", len_getter, None);
        let bl_getter = self.native_method("get byteLength", 0, |interp, this, _args| {
            let ta = interp.typed_array_object(this)?;
            let n = interp.typed_array_len(&ta) * interp.typed_array_kind(&ta).element_size();
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&proto, "byteLength", bl_getter, None);
        let bo_getter = self.native_method("get byteOffset", 0, |interp, this, _args| {
            let ta = interp.typed_array_object(this)?;
            let n = interp.typed_array_offset(&ta);
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&proto, "byteOffset", bo_getter, None);
        let buffer_getter = self.native_method("get buffer", 0, |interp, this, _args| {
            let ta = interp.typed_array_object(this)?;
            let b = interp.typed_array_buffer(&ta);
            Ok(Value::Object(b))
        });
        self.define_accessor_public(&proto, "buffer", buffer_getter, None);

        self.define_method(&proto, "set", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let source = args.first().cloned().unwrap_or(Value::Undefined);
            let offset = match args.get(1) {
                Some(v) if !v.is_undefined() => interp.to_index(v)?,
                _ => 0,
            };
            let src_obj = interp.to_object(&source)?;
            let len = interp.length_of_array_like(&src_obj)? as usize;
            let target_len = interp.typed_array_len(&ta);
            if offset + len > target_len {
                return interp.throw_range_error("Source is too large");
            }
            for i in 0..len {
                let v = interp.get_property(
                    &Value::Object(src_obj.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                let n = interp.to_number(&v)?;
                interp.typed_array_set(&ta, offset + i, n);
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "subarray", 2, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let start = clamp_rel(interp, args.first(), len, 0)?;
            let end = clamp_rel(interp, args.get(1), len, len)?;
            let buffer = interp.typed_array_buffer(&ta);
            let kind = interp.typed_array_kind(&ta);
            let byte_offset = interp.typed_array_offset(&ta) + start * kind.element_size();
            let new_len = end.saturating_sub(start);
            let proto = interp
                .typed_array_prototype_for_kind(kind)
                .unwrap_or_else(|| interp.realm.intrinsics.typed_array_prototype.clone());
            Ok(Value::Object(interp.make_typed_array_view(
                kind,
                buffer,
                byte_offset,
                new_len,
                proto,
            )))
        });
        self.define_method(&proto, "slice", 2, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let start = clamp_rel(interp, args.first(), len, 0)?;
            let end = clamp_rel(interp, args.get(1), len, len)?;
            let kind = interp.typed_array_kind(&ta);
            let count = end.saturating_sub(start);
            let buffer = interp.alloc_array_buffer(count * kind.element_size(), false);
            let buf_obj = buffer.clone();
            let proto = interp
                .typed_array_prototype_for_kind(kind)
                .unwrap_or_else(|| interp.realm.intrinsics.typed_array_prototype.clone());
            let result = interp.make_typed_array_view(kind, buf_obj, 0, count, proto);
            for i in 0..count {
                let v = interp.typed_array_get(&ta, start + i);
                interp.typed_array_set(&result, i, v);
            }
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "fill", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let value = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            let start = clamp_rel(interp, args.get(1), len, 0)?;
            let end = clamp_rel(interp, args.get(2), len, len)?;
            for i in start..end {
                interp.typed_array_set(&ta, i, value);
            }
            Ok(Value::Object(ta))
        });
        self.define_method(&proto, "copyWithin", 2, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let target = clamp_rel(interp, args.first(), len, 0)?;
            let start = clamp_rel(interp, args.get(1), len, 0)?;
            let end = clamp_rel(interp, args.get(2), len, len)?;
            let count = (end - start).min(len - target);
            let mut vals = Vec::new();
            for i in 0..count {
                vals.push(interp.typed_array_get(&ta, start + i));
            }
            for (i, v) in vals.into_iter().enumerate() {
                interp.typed_array_set(&ta, target + i, v);
            }
            Ok(Value::Object(ta))
        });
        self.define_method(&proto, "reverse", 0, |interp, this, _args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            for i in 0..len / 2 {
                let j = len - 1 - i;
                let a = interp.typed_array_get(&ta, i);
                let b = interp.typed_array_get(&ta, j);
                interp.typed_array_set(&ta, i, b);
                interp.typed_array_set(&ta, j, a);
            }
            Ok(Value::Object(ta))
        });
        self.define_method(&proto, "indexOf", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            for i in 0..len {
                let v = interp.typed_array_get(&ta, i);
                if Value::strict_equals(&Value::Number(v), &target) {
                    return Ok(Value::Number(i as f64));
                }
            }
            Ok(Value::Number(-1.0))
        });
        self.define_method(&proto, "lastIndexOf", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            for i in (0..len).rev() {
                let v = interp.typed_array_get(&ta, i);
                if Value::strict_equals(&Value::Number(v), &target) {
                    return Ok(Value::Number(i as f64));
                }
            }
            Ok(Value::Number(-1.0))
        });
        self.define_method(&proto, "includes", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            for i in 0..len {
                let v = interp.typed_array_get(&ta, i);
                if Value::same_value_zero(&Value::Number(v), &target) {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        });
        self.define_method(&proto, "forEach", 1, |interp, this, args| {
            typed_foreach(interp, this, args)
        });
        self.define_method(&proto, "every", 1, |interp, this, args| {
            typed_predicate(interp, this, args, true)
        });
        self.define_method(&proto, "some", 1, |interp, this, args| {
            typed_predicate(interp, this, args, false)
        });
        self.define_method(&proto, "find", 1, |interp, this, args| {
            typed_find(interp, this, args, false)
        });
        self.define_method(&proto, "findIndex", 1, |interp, this, args| {
            typed_find(interp, this, args, true)
        });
        self.define_method(&proto, "join", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let sep = match args.first() {
                Some(v) if !v.is_undefined() => interp.to_string(v)?,
                _ => JsString::from_str(","),
            };
            let mut out = JsString::empty();
            for i in 0..len {
                if i > 0 {
                    out = out.concat(&sep);
                }
                let v = interp.typed_array_get(&ta, i);
                out = out.concat(&interp.to_string(&Value::Number(v))?);
            }
            Ok(Value::String(out))
        });
        self.define_method(&proto, "toString", 0, |interp, this, args| {
            let join = interp.get_property(this, &PropertyKey::from_str("join"))?;
            interp.call_function(&join, this, args)
        });
        self.define_method(&proto, "map", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let kind = interp.typed_array_kind(&ta);
            let buffer = interp.alloc_array_buffer(len * kind.element_size(), false);
            let proto = interp
                .typed_array_prototype_for_kind(kind)
                .unwrap_or_else(|| interp.realm.intrinsics.typed_array_prototype.clone());
            let result = interp.make_typed_array_view(kind, buffer, 0, len, proto);
            for i in 0..len {
                let v = interp.typed_array_get(&ta, i);
                let mapped = interp.call_function(
                    &cb,
                    &this_arg,
                    &[
                        Value::Number(v),
                        Value::Number(i as f64),
                        Value::Object(ta.clone()),
                    ],
                )?;
                let n = interp.to_number(&mapped)?;
                interp.typed_array_set(&result, i, n);
            }
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "filter", 1, |interp, this, args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let mut vals = Vec::new();
            for i in 0..len {
                let v = interp.typed_array_get(&ta, i);
                let keep = interp.call_function(
                    &cb,
                    &this_arg,
                    &[
                        Value::Number(v),
                        Value::Number(i as f64),
                        Value::Object(ta.clone()),
                    ],
                )?;
                if keep.truthy() {
                    vals.push(v);
                }
            }
            let kind = interp.typed_array_kind(&ta);
            let buffer = interp.alloc_array_buffer(vals.len() * kind.element_size(), false);
            let proto = interp
                .typed_array_prototype_for_kind(kind)
                .unwrap_or_else(|| interp.realm.intrinsics.typed_array_prototype.clone());
            let result = interp.make_typed_array_view(kind, buffer, 0, vals.len(), proto);
            for (i, v) in vals.into_iter().enumerate() {
                interp.typed_array_set(&result, i, v);
            }
            Ok(Value::Object(result))
        });
        self.define_method(&proto, "values", 0, |interp, this, _args| {
            let ta = interp.typed_array_object(this)?;
            let len = interp.typed_array_len(&ta);
            let vals: Vec<Value> = (0..len)
                .map(|i| Value::Number(interp.typed_array_get(&ta, i)))
                .collect();
            let arr = interp.create_array_from_list(vals)?;
            let values = interp.get_property(
                &Value::Object(arr.clone()),
                &PropertyKey::from_str("values"),
            )?;
            interp.call_function(&values, &Value::Object(arr), &[])
        });
        let values = self
            .get_property(
                &Value::Object(proto.clone()),
                &PropertyKey::from_str("values"),
            )
            .ok();
        if let Some(v) = values {
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
        let _ = len_getter;
        let tag_getter =
            self.native_method("get [Symbol.toStringTag]", 0, |interp, this, _args| {
                if let Some(o) = this.as_object_cloned() {
                    if let ObjectKind::TypedArray(t) = &o.borrow().kind {
                        return Ok(Value::String(JsString::from_str(t.kind.name())));
                    }
                }
                let _ = interp;
                Ok(Value::Undefined)
            });
        self.define_accessor_public(&proto, "[Symbol.toStringTag]", tag_getter, None);
        let abstract_ctor = crate::builtins::native_ctor(
            "TypedArray",
            0,
            |interp, _this, _args| {
                interp.throw_type_error("Abstract class TypedArray not directly constructible")
            },
            |interp, _args, _nt| {
                interp.throw_type_error("Abstract class TypedArray not directly constructible")
            },
        );
        abstract_ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        abstract_ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(abstract_ctor.clone()));
        self.realm
            .global
            .put_hidden("TypedArray", Value::Object(abstract_ctor));
    }

    fn install_typed_array_ctors(&mut self) {
        let kinds = [
            TypedArrayKind::Int8,
            TypedArrayKind::Uint8,
            TypedArrayKind::Uint8Clamped,
            TypedArrayKind::Int16,
            TypedArrayKind::Uint16,
            TypedArrayKind::Int32,
            TypedArrayKind::Uint32,
            TypedArrayKind::Float32,
            TypedArrayKind::Float64,
            TypedArrayKind::BigInt64,
            TypedArrayKind::BigUint64,
        ];
        for kind in kinds {
            let name = kind.name();
            let proto =
                JsObject::ordinary(Some(self.realm.intrinsics.typed_array_prototype.clone()));
            let ctor = crate::builtins::native_ctor(
                name,
                3,
                |interp, _this, _args| interp.throw_type_error("Constructor requires 'new'"),
                move |interp, args, new_target| {
                    interp.construct_typed_array(kind, args, new_target)
                },
            );
            let abstract_base = self
                .realm
                .global
                .get_hidden("TypedArray")
                .and_then(|v| v.as_object_cloned())
                .unwrap_or_else(|| self.realm.intrinsics.function_prototype.clone());
            ctor.set_prototype_raw(Some(abstract_base));
            ctor.define_fixed("prototype", Value::Object(proto.clone()));
            ctor.define_fixed(
                "BYTES_PER_ELEMENT",
                Value::Number(kind.element_size() as f64),
            );
            proto.put_hidden("constructor", Value::Object(ctor.clone()));
            proto.put_hidden(
                "BYTES_PER_ELEMENT",
                Value::Number(kind.element_size() as f64),
            );
            self.realm
                .global
                .put_hidden(name, Value::Object(ctor.clone()));
            let ctor_from = ctor.clone();
            self.define_method(&ctor, "from", 1, move |interp, _this, args| {
                let source = args.first().cloned().unwrap_or(Value::Undefined);
                let map_fn = args.get(1).cloned().unwrap_or(Value::Undefined);
                let this_arg = args.get(2).cloned().unwrap_or(Value::Undefined);
                let mut vals = if source.is_nullish() {
                    Vec::new()
                } else {
                    let iter_method = interp.get_method(
                        &source,
                        &PropertyKey::Symbol(interp.realm.intrinsics.symbol_iterator.clone()),
                    )?;
                    if iter_method.is_some() {
                        interp.iterable_to_list(&source)?
                    } else {
                        let obj = interp.to_object(&source)?;
                        let len = interp.length_of_array_like(&obj)? as usize;
                        let mut v = Vec::new();
                        for i in 0..len {
                            v.push(interp.get_property(
                                &Value::Object(obj.clone()),
                                &PropertyKey::String(JsString::from_str(&i.to_string())),
                            )?);
                        }
                        v
                    }
                };
                if interp.is_callable_value(&map_fn) {
                    let mut mapped = Vec::new();
                    for (i, v) in vals.into_iter().enumerate() {
                        mapped.push(interp.call_function(
                            &map_fn,
                            &this_arg,
                            &[v, Value::Number(i as f64)],
                        )?);
                    }
                    vals = mapped;
                }
                let arr = interp.create_array_from_list(vals)?;
                interp.construct_typed_array(kind, &[Value::Object(arr)], &ctor_from)
            });
            let ctor_of = ctor.clone();
            self.define_method(&ctor, "of", 0, move |interp, _this, args| {
                let arr = interp.create_array_from_list(args.to_vec())?;
                interp.construct_typed_array(kind, &[Value::Object(arr)], &ctor_of)
            });
            self.realm.intrinsics.typed_array_ctors.push((kind, ctor));
        }
    }

    fn install_data_view(&mut self) {
        let proto = self.realm.intrinsics.data_view_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "DataView",
            1,
            |interp, _this, _args| interp.throw_type_error("Constructor DataView requires 'new'"),
            |interp, args, new_target| {
                let buffer = match args.first().and_then(|v| v.as_object_cloned()) {
                    Some(o) => o,
                    None => {
                        return interp
                            .throw_type_error("First argument to DataView must be an ArrayBuffer")
                    }
                };
                interp.array_buffer_object(&Value::Object(buffer.clone()))?;
                let len = interp.array_buffer_len(&buffer);
                let offset = match args.get(1) {
                    Some(v) if !v.is_undefined() => interp.to_index(v)?,
                    _ => 0,
                };
                let byte_len = match args.get(2) {
                    Some(v) if !v.is_undefined() => interp.to_index(v)?,
                    _ => len.saturating_sub(offset),
                };
                if offset + byte_len > len {
                    return interp.throw_range_error("DataView is out of bounds");
                }
                let p = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::DataView(DataViewData {
                        buffer,
                        byte_offset: offset,
                        byte_length: byte_len,
                    }),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("DataView", Value::Object(ctor.clone()));
        self.realm.intrinsics.data_view_ctor = Some(ctor);
        let buffer_getter = self.native_method("get buffer", 0, |interp, this, _args| {
            let dv = interp.data_view_object(this)?;
            let b = interp.data_view_buffer(&dv);
            Ok(Value::Object(b))
        });
        self.define_accessor_public(&proto, "buffer", buffer_getter, None);
        let bl = self.native_method("get byteLength", 0, |interp, this, _args| {
            let dv = interp.data_view_object(this)?;
            Ok(Value::Number(interp.data_view_len(&dv) as f64))
        });
        self.define_accessor_public(&proto, "byteLength", bl, None);
        let bo = self.native_method("get byteOffset", 0, |interp, this, _args| {
            let dv = interp.data_view_object(this)?;
            Ok(Value::Number(interp.data_view_offset(&dv) as f64))
        });
        self.define_accessor_public(&proto, "byteOffset", bo, None);

        for (name, size, signed, float, big) in [
            ("Int8", 1, true, false, false),
            ("Uint8", 1, false, false, false),
            ("Int16", 2, true, false, false),
            ("Uint16", 2, false, false, false),
            ("Int32", 4, true, false, false),
            ("Uint32", 4, false, false, false),
            ("Float32", 4, false, true, false),
            ("Float64", 8, false, true, false),
            ("BigInt64", 8, true, false, true),
            ("BigUint64", 8, false, false, true),
        ] {
            let get_name = format!("get{}", name);
            self.define_method(&proto, &get_name, 1, move |interp, this, args| {
                let dv = interp.data_view_object(this)?;
                let idx = interp.to_index(args.first().unwrap_or(&Value::Undefined))?;
                let little = args.get(1).map(|v| v.truthy()).unwrap_or(false);
                if idx + size > interp.data_view_len(&dv) {
                    return interp
                        .throw_range_error("Offset is outside the bounds of the DataView");
                }
                let bytes = interp.data_view_read(&dv, idx, size, little);
                if big {
                    let v = if signed {
                        sign_extend(&bytes) as i64
                    } else {
                        u64::from_le_bytes(pad_u64(&bytes, little)) as i64
                    };
                    Ok(Value::BigInt(Rc::new(num_bigint::BigInt::from(v))))
                } else if float {
                    let v = if size == 4 {
                        f32::from_bits(u32::from_le_bytes(pad_u32(&bytes, little))) as f64
                    } else {
                        f64::from_bits(u64::from_le_bytes(pad_u64(&bytes, little)))
                    };
                    Ok(Value::Number(v))
                } else {
                    let v = read_int(&bytes, signed);
                    Ok(Value::Number(v as f64))
                }
            });
            let set_name = format!("set{}", name);
            self.define_method(&proto, &set_name, 2, move |interp, this, args| {
                let dv = interp.data_view_object(this)?;
                let idx = interp.to_index(args.first().unwrap_or(&Value::Undefined))?;
                let value = args.get(1).cloned().unwrap_or(Value::Undefined);
                let little = args.get(2).map(|v| v.truthy()).unwrap_or(false);
                if idx + size > interp.data_view_len(&dv) {
                    return interp
                        .throw_range_error("Offset is outside the bounds of the DataView");
                }
                let bytes = if big {
                    let b = interp.to_bigint(&value)?;
                    let n = b.to_u64().unwrap_or(0);
                    if little {
                        n.to_le_bytes().to_vec()
                    } else {
                        n.to_be_bytes().to_vec()
                    }
                } else if float {
                    let n = interp.to_number(&value)?;
                    if size == 4 {
                        (n as f32).to_bits().to_le_bytes().to_vec()
                    } else {
                        n.to_bits().to_le_bytes().to_vec()
                    }
                } else {
                    let n = interp.to_number(&value)?;
                    let v = crate::ops::f64_to_uint32(n);
                    let full = if size == 1 {
                        vec![v as u8]
                    } else if size == 2 {
                        (v as u16).to_le_bytes().to_vec()
                    } else {
                        v.to_le_bytes().to_vec()
                    };
                    full
                };
                interp.data_view_write(&dv, idx, &bytes, little);
                Ok(Value::Undefined)
            });
        }
    }

    fn construct_typed_array(
        &mut self,
        kind: TypedArrayKind,
        args: &[Value],
        new_target: &JsObject,
    ) -> JsResult<Value> {
        let proto = self.prototype_from_new_target(new_target);
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        if let Some(o) = first.as_object_cloned() {
            let is_buffer = matches!(
                &o.borrow().kind,
                ObjectKind::ArrayBuffer(_) | ObjectKind::SharedArrayBuffer(_)
            );
            if is_buffer {
                let buffer_len = self.array_buffer_len(&o);
                let offset = match args.get(1) {
                    Some(v) if !v.is_undefined() => self.to_index(v)?,
                    _ => 0,
                };
                if offset % kind.element_size() != 0 {
                    return self.throw_range_error("Start offset is not aligned");
                }
                let length = match args.get(2) {
                    Some(v) if !v.is_undefined() => self.to_index(v)?,
                    _ => (buffer_len.saturating_sub(offset)) / kind.element_size(),
                };
                if offset + length * kind.element_size() > buffer_len {
                    return self.throw_range_error("Invalid typed array length");
                }
                return Ok(Value::Object(
                    self.make_typed_array_view(kind, o, offset, length, proto),
                ));
            }
        }
        let values: Vec<Value> = if first.is_undefined() {
            Vec::new()
        } else if let Some(n) = first.as_number() {
            let len = if n < 0.0 || n.fract() != 0.0 || n.is_infinite() {
                return self.throw_range_error("Invalid typed array length");
            } else {
                n as usize
            };
            let buffer = self.alloc_array_buffer(len * kind.element_size(), false);
            return Ok(Value::Object(
                self.make_typed_array_view(kind, buffer, 0, len, proto),
            ));
        } else {
            let mut vals = Vec::new();
            let iter_method = self.get_method(
                &first,
                &PropertyKey::Symbol(self.realm.intrinsics.symbol_iterator.clone()),
            )?;
            if iter_method.is_some() {
                vals = self.iterable_to_list(&first)?;
            } else {
                let obj = self.to_object(&first)?;
                let len = self.length_of_array_like(&obj)? as usize;
                for i in 0..len {
                    vals.push(self.get_property(
                        &Value::Object(obj.clone()),
                        &PropertyKey::String(JsString::from_str(&i.to_string())),
                    )?);
                }
            }
            vals
        };
        let buffer = self.alloc_array_buffer(values.len() * kind.element_size(), false);
        let result = self.make_typed_array_view(kind, buffer, 0, values.len(), proto);
        for (i, v) in values.iter().enumerate() {
            if kind.is_bigint() {
                let b = self.to_bigint(v)?;
                if let ObjectKind::TypedArray(t) = &result.borrow().kind {
                    t.set_element_bigint(i, &b);
                }
            } else {
                let n = self.to_number(v)?;
                self.typed_array_set(&result, i, n);
            }
        }
        Ok(Value::Object(result))
    }

    pub fn make_typed_array_view(
        &mut self,
        kind: TypedArrayKind,
        buffer: JsObject,
        byte_offset: usize,
        length: usize,
        proto: JsObject,
    ) -> JsObject {
        JsObject::new(ObjectData {
            prototype: Some(proto),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::TypedArray(TypedArrayData {
                buffer,
                byte_offset,
                length,
                kind,
                tracked: false,
            }),
            private: IndexMap::new(),
        })
    }

    pub fn alloc_array_buffer(&mut self, len: usize, shared: bool) -> JsObject {
        JsObject::new(ObjectData {
            prototype: Some(if shared {
                self.realm.intrinsics.shared_array_buffer_prototype.clone()
            } else {
                self.realm.intrinsics.array_buffer_prototype.clone()
            }),
            extensible: true,
            properties: IndexMap::new(),
            kind: if shared {
                ObjectKind::SharedArrayBuffer(ArrayBufferData {
                    bytes: Rc::new(RefCell::new(vec![0u8; len])),
                    detached: false,
                    max_byte_length: None,
                    resizable: false,
                })
            } else {
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: Rc::new(RefCell::new(vec![0u8; len])),
                    detached: false,
                    max_byte_length: None,
                    resizable: false,
                })
            },
            private: IndexMap::new(),
        })
    }

    pub fn typed_array_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::TypedArray(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("this is not a typed array")
    }
    pub fn array_buffer_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(
                &o.borrow().kind,
                ObjectKind::ArrayBuffer(_) | ObjectKind::SharedArrayBuffer(_)
            ) {
                return Ok(o);
            }
        }
        self.throw_type_error("this is not an ArrayBuffer")
    }
    pub fn data_view_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::DataView(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("this is not a DataView")
    }
    pub fn array_buffer_len(&self, o: &JsObject) -> usize {
        match &o.borrow().kind {
            ObjectKind::ArrayBuffer(a) | ObjectKind::SharedArrayBuffer(a) => a.bytes.borrow().len(),
            _ => 0,
        }
    }
    pub fn array_buffer_bytes(&self, o: &JsObject) -> Vec<u8> {
        match &o.borrow().kind {
            ObjectKind::ArrayBuffer(a) | ObjectKind::SharedArrayBuffer(a) => {
                a.bytes.borrow().clone()
            }
            _ => Vec::new(),
        }
    }
    pub fn typed_array_len(&self, o: &JsObject) -> usize {
        match &o.borrow().kind {
            ObjectKind::TypedArray(t) => t.length,
            _ => 0,
        }
    }
    pub fn typed_array_offset(&self, o: &JsObject) -> usize {
        match &o.borrow().kind {
            ObjectKind::TypedArray(t) => t.byte_offset,
            _ => 0,
        }
    }
    pub fn typed_array_kind(&self, o: &JsObject) -> TypedArrayKind {
        match &o.borrow().kind {
            ObjectKind::TypedArray(t) => t.kind,
            _ => TypedArrayKind::Uint8,
        }
    }
    pub fn typed_array_buffer(&self, o: &JsObject) -> JsObject {
        match &o.borrow().kind {
            ObjectKind::TypedArray(t) => t.buffer.clone(),
            _ => self.realm.global.clone(),
        }
    }
    pub fn typed_array_get(&self, o: &JsObject, idx: usize) -> f64 {
        match &o.borrow().kind {
            ObjectKind::TypedArray(t) => t.get_element(idx),
            _ => 0.0,
        }
    }
    pub fn typed_array_set(&self, o: &JsObject, idx: usize, value: f64) {
        if let ObjectKind::TypedArray(t) = &o.borrow().kind {
            t.set_element(idx, value);
        }
    }
    fn typed_array_prototype_for_kind(&mut self, kind: TypedArrayKind) -> Option<JsObject> {
        let ctor = self
            .realm
            .intrinsics
            .typed_array_ctors
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, c)| c.clone())?;
        Some(
            self.get_property(&Value::Object(ctor), &PropertyKey::from_str("prototype"))
                .ok()
                .and_then(|v| v.as_object_cloned())
                .unwrap_or_else(|| self.realm.intrinsics.typed_array_prototype.clone()),
        )
    }
    pub fn data_view_len(&self, o: &JsObject) -> usize {
        match &o.borrow().kind {
            ObjectKind::DataView(d) => d.byte_length,
            _ => 0,
        }
    }
    pub fn data_view_offset(&self, o: &JsObject) -> usize {
        match &o.borrow().kind {
            ObjectKind::DataView(d) => d.byte_offset,
            _ => 0,
        }
    }
    pub fn data_view_buffer(&self, o: &JsObject) -> JsObject {
        match &o.borrow().kind {
            ObjectKind::DataView(d) => d.buffer.clone(),
            _ => self.realm.global.clone(),
        }
    }
    pub fn data_view_read(&self, o: &JsObject, idx: usize, size: usize, little: bool) -> Vec<u8> {
        let (buffer, base) = match &o.borrow().kind {
            ObjectKind::DataView(d) => (d.buffer.clone(), d.byte_offset + idx),
            _ => return vec![0; size],
        };
        let bytes = self.array_buffer_bytes(&buffer);
        let mut out = bytes[base..base + size].to_vec();
        if !little {
            out.reverse();
        }
        out
    }
    pub fn data_view_write(&self, o: &JsObject, idx: usize, bytes: &[u8], little: bool) {
        let (buffer, base) = match &o.borrow().kind {
            ObjectKind::DataView(d) => (d.buffer.clone(), d.byte_offset + idx),
            _ => return,
        };
        let bytes_rc = match &buffer.borrow().kind {
            ObjectKind::ArrayBuffer(a) | ObjectKind::SharedArrayBuffer(a) => a.bytes.clone(),
            _ => return,
        };
        let mut data = bytes_rc.borrow_mut();
        if little {
            data[base..base + bytes.len()].copy_from_slice(bytes);
        } else {
            for (i, b) in bytes.iter().rev().enumerate() {
                data[base + i] = *b;
            }
        }
    }
}

fn clamp_rel(
    interp: &mut Interp,
    v: Option<&Value>,
    len: usize,
    default: usize,
) -> JsResult<usize> {
    match v {
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

fn typed_foreach(interp: &mut Interp, this: &Value, args: &[Value]) -> JsResult<Value> {
    let ta = interp.typed_array_object(this)?;
    let len = interp.typed_array_len(&ta);
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
    for i in 0..len {
        let v = interp.typed_array_get(&ta, i);
        interp.call_function(
            &cb,
            &this_arg,
            &[
                Value::Number(v),
                Value::Number(i as f64),
                Value::Object(ta.clone()),
            ],
        )?;
    }
    Ok(Value::Undefined)
}

fn typed_predicate(
    interp: &mut Interp,
    this: &Value,
    args: &[Value],
    every: bool,
) -> JsResult<Value> {
    let ta = interp.typed_array_object(this)?;
    let len = interp.typed_array_len(&ta);
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
    for i in 0..len {
        let v = interp.typed_array_get(&ta, i);
        let r = interp.call_function(
            &cb,
            &this_arg,
            &[
                Value::Number(v),
                Value::Number(i as f64),
                Value::Object(ta.clone()),
            ],
        )?;
        if every && !r.truthy() {
            return Ok(Value::Bool(false));
        }
        if !every && r.truthy() {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(every))
}

fn typed_find(interp: &mut Interp, this: &Value, args: &[Value], index: bool) -> JsResult<Value> {
    let ta = interp.typed_array_object(this)?;
    let len = interp.typed_array_len(&ta);
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
    for i in 0..len {
        let v = interp.typed_array_get(&ta, i);
        let r = interp.call_function(
            &cb,
            &this_arg,
            &[
                Value::Number(v),
                Value::Number(i as f64),
                Value::Object(ta.clone()),
            ],
        )?;
        if r.truthy() {
            return Ok(if index {
                Value::Number(i as f64)
            } else {
                Value::Number(v)
            });
        }
    }
    Ok(if index {
        Value::Number(-1.0)
    } else {
        Value::Undefined
    })
}

fn pad_u32(bytes: &[u8], _little: bool) -> [u8; 4] {
    let mut out = [0u8; 4];
    for (i, b) in bytes.iter().take(4).enumerate() {
        out[i] = *b;
    }
    out
}
fn pad_u64(bytes: &[u8], _little: bool) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (i, b) in bytes.iter().take(8).enumerate() {
        out[i] = *b;
    }
    out
}
fn sign_extend(bytes: &[u8]) -> u64 {
    let mut v: u64 = 0;
    for (i, b) in bytes.iter().enumerate() {
        v |= (*b as u64) << (8 * i);
    }
    v
}
fn read_int(bytes: &[u8], signed: bool) -> i64 {
    let mut v: u64 = 0;
    for (i, b) in bytes.iter().enumerate() {
        v |= (*b as u64) << (8 * i);
    }
    if signed {
        match bytes.len() {
            1 => v as u8 as i8 as i64,
            2 => v as u16 as i16 as i64,
            4 => v as u32 as i32 as i64,
            _ => v as i64,
        }
    } else {
        v as i64
    }
}
