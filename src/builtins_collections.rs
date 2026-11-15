//! Map, Set, WeakMap and WeakSet.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

impl Interp {
    pub fn install_collections(&mut self) {
        self.install_map();
        self.install_set();
        self.install_weak();
        self.install_iterator_protos();
    }

    fn install_iterator_protos(&mut self) {
        let protos = [
            (
                "Array Iterator",
                self.realm.intrinsics.array_iterator_prototype.clone(),
            ),
            (
                "String Iterator",
                self.realm.intrinsics.string_iterator_prototype.clone(),
            ),
            (
                "Map Iterator",
                self.realm.intrinsics.map_iterator_prototype.clone(),
            ),
            (
                "Set Iterator",
                self.realm.intrinsics.set_iterator_prototype.clone(),
            ),
        ];
        for (name, proto) in protos {
            let iter = self.native_method("[Symbol.iterator]", 0, |_interp, this, _args| {
                Ok(this.clone())
            });
            let desc = Descriptor {
                value: Some(Value::Object(iter)),
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
            let tag = Descriptor {
                value: Some(Value::String(JsString::from_str(name))),
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
        }
    }

    fn install_map(&mut self) {
        let proto = self.realm.intrinsics.map_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "Map",
            0,
            |interp, _this, _args| interp.throw_type_error("Constructor Map requires 'new'"),
            |interp, args, new_target| {
                let p = interp.prototype_from_new_target(new_target);
                let map = JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::Map(MapData {
                        entries: Vec::new(),
                    }),
                    private: IndexMap::new(),
                });
                if let Some(iterable) = args.first() {
                    if !iterable.is_nullish() {
                        let iter = interp.get_iterator(iterable)?;
                        loop {
                            let (done, v) = interp.iterator_next(&iter)?;
                            if done {
                                break;
                            }
                            let entry = interp.to_object(&v)?;
                            let k = interp.get_property(
                                &Value::Object(entry.clone()),
                                &PropertyKey::from_str("0"),
                            )?;
                            let val = interp
                                .get_property(&Value::Object(entry), &PropertyKey::from_str("1"))?;
                            interp.map_set(&map, k, val, true)?;
                        }
                    }
                }
                Ok(Value::Object(map))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("Map", Value::Object(ctor.clone()));
        self.realm.intrinsics.map_ctor = Some(ctor);

        self.define_method(&proto, "get", 1, |interp, this, args| {
            let map = interp.map_object(this)?;
            Ok(interp.map_get(&map, args.first().unwrap_or(&Value::Undefined)))
        });
        self.define_method(&proto, "set", 2, |interp, this, args| {
            let map = interp.map_object(this)?;
            let k = args.first().cloned().unwrap_or(Value::Undefined);
            let v = args.get(1).cloned().unwrap_or(Value::Undefined);
            interp.map_set(&map, k, v, false)?;
            Ok(Value::Object(map))
        });
        self.define_method(&proto, "has", 1, |interp, this, args| {
            let map = interp.map_object(this)?;
            Ok(Value::Bool(
                interp.map_has(&map, args.first().unwrap_or(&Value::Undefined)),
            ))
        });
        self.define_method(&proto, "delete", 1, |interp, this, args| {
            let map = interp.map_object(this)?;
            Ok(Value::Bool(interp.map_delete(
                &map,
                args.first().unwrap_or(&Value::Undefined),
            )))
        });
        self.define_method(&proto, "clear", 0, |interp, this, _args| {
            let map = interp.map_object(this)?;
            if let ObjectKind::Map(m) = &mut map.0.borrow_mut().kind {
                m.entries.clear();
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "forEach", 1, |interp, this, args| {
            let map = interp.map_object(this)?;
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            if !interp.is_callable_value(&cb) {
                return interp.throw_type_error("callback is not a function");
            }
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let entries = interp.map_entries(&map);
            for (k, v) in entries {
                interp.call_function(&cb, &this_arg, &[v, k, Value::Object(map.clone())])?;
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "keys", 0, |interp, this, _args| {
            let map = interp.map_object(this)?;
            make_collection_iterator(interp, &map, CollectionIterKind::MapKey)
        });
        self.define_method(&proto, "values", 0, |interp, this, _args| {
            let map = interp.map_object(this)?;
            make_collection_iterator(interp, &map, CollectionIterKind::MapValue)
        });
        self.define_method(&proto, "entries", 0, |interp, this, _args| {
            let map = interp.map_object(this)?;
            make_collection_iterator(interp, &map, CollectionIterKind::MapEntry)
        });
        let entries = self
            .get_property(
                &Value::Object(proto.clone()),
                &PropertyKey::from_str("entries"),
            )
            .ok();
        if let Some(v) = entries {
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
        let size_getter = self.native_method("get size", 0, |interp, this, _args| {
            let map = interp.map_object(this)?;
            let n = interp.map_entries(&map).len();
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&proto, "size", size_getter, None);
        let tag = Descriptor {
            value: Some(Value::String(JsString::from_str("Map"))),
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
    }

    fn install_set(&mut self) {
        let proto = self.realm.intrinsics.set_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "Set",
            0,
            |interp, _this, _args| interp.throw_type_error("Constructor Set requires 'new'"),
            |interp, args, new_target| {
                let p = interp.prototype_from_new_target(new_target);
                let set = JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::Set(SetData {
                        entries: Vec::new(),
                    }),
                    private: IndexMap::new(),
                });
                if let Some(iterable) = args.first() {
                    if !iterable.is_nullish() {
                        let iter = interp.get_iterator(iterable)?;
                        loop {
                            let (done, v) = interp.iterator_next(&iter)?;
                            if done {
                                break;
                            }
                            interp.set_add(&set, v);
                        }
                    }
                }
                Ok(Value::Object(set))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(proto.clone()));
        proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("Set", Value::Object(ctor.clone()));
        self.realm.intrinsics.set_ctor = Some(ctor);

        self.define_method(&proto, "add", 1, |interp, this, args| {
            let set = interp.set_object(this)?;
            interp.set_add(&set, args.first().cloned().unwrap_or(Value::Undefined));
            Ok(Value::Object(set))
        });
        self.define_method(&proto, "has", 1, |interp, this, args| {
            let set = interp.set_object(this)?;
            Ok(Value::Bool(
                interp.set_has(&set, args.first().unwrap_or(&Value::Undefined)),
            ))
        });
        self.define_method(&proto, "delete", 1, |interp, this, args| {
            let set = interp.set_object(this)?;
            Ok(Value::Bool(interp.set_delete(
                &set,
                args.first().unwrap_or(&Value::Undefined),
            )))
        });
        self.define_method(&proto, "clear", 0, |interp, this, _args| {
            let set = interp.set_object(this)?;
            if let ObjectKind::Set(s) = &mut set.0.borrow_mut().kind {
                s.entries.clear();
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "forEach", 1, |interp, this, args| {
            let set = interp.set_object(this)?;
            let cb = args.first().cloned().unwrap_or(Value::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let vals = interp.set_entries(&set);
            for v in vals {
                interp.call_function(
                    &cb,
                    &this_arg,
                    &[v.clone(), v, Value::Object(set.clone())],
                )?;
            }
            Ok(Value::Undefined)
        });
        self.define_method(&proto, "values", 0, |interp, this, _args| {
            let set = interp.set_object(this)?;
            make_collection_iterator(interp, &set, CollectionIterKind::SetValue)
        });
        self.define_method(&proto, "keys", 0, |interp, this, _args| {
            let set = interp.set_object(this)?;
            make_collection_iterator(interp, &set, CollectionIterKind::SetValue)
        });
        self.define_method(&proto, "entries", 0, |interp, this, _args| {
            let set = interp.set_object(this)?;
            make_collection_iterator(interp, &set, CollectionIterKind::SetEntry)
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
        let size_getter = self.native_method("get size", 0, |interp, this, _args| {
            let set = interp.set_object(this)?;
            let n = interp.set_entries(&set).len();
            Ok(Value::Number(n as f64))
        });
        self.define_accessor_public(&proto, "size", size_getter, None);
        let tag = Descriptor {
            value: Some(Value::String(JsString::from_str("Set"))),
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
    }

    fn install_weak(&mut self) {
        let weak_map_proto = self.realm.intrinsics.weak_map_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "WeakMap",
            0,
            |interp, _this, _args| interp.throw_type_error("Constructor WeakMap requires 'new'"),
            |interp, _args, new_target| {
                let p = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::WeakMap(WeakMapData {
                        entries: Vec::new(),
                    }),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(weak_map_proto.clone()));
        weak_map_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("WeakMap", Value::Object(ctor.clone()));
        self.realm.intrinsics.weak_map_ctor = Some(ctor);
        self.define_method(&weak_map_proto, "get", 1, |interp, this, args| {
            let o = interp.weak_map_object(this)?;
            let key = args.first().and_then(|v| v.as_object_cloned());
            let key = match key {
                Some(k) => k,
                None => return Ok(Value::Undefined),
            };
            let data = o.0.borrow();
            if let ObjectKind::WeakMap(m) = &data.kind {
                for (k, v) in &m.entries {
                    if k.ptr_eq(&key) {
                        return Ok(v.clone());
                    }
                }
            }
            Ok(Value::Undefined)
        });
        self.define_method(&weak_map_proto, "set", 2, |interp, this, args| {
            let o = interp.weak_map_object(this)?;
            let key = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(k) => k,
                None => return interp.throw_type_error("Invalid value used as weak map key"),
            };
            let value = args.get(1).cloned().unwrap_or(Value::Undefined);
            let mut data = o.0.borrow_mut();
            if let ObjectKind::WeakMap(m) = &mut data.kind {
                for (k, v) in &mut m.entries {
                    if k.ptr_eq(&key) {
                        *v = value;
                        return Ok(Value::Object(o.clone()));
                    }
                }
                m.entries.push((key, value));
            }
            drop(data);
            Ok(Value::Object(o))
        });
        self.define_method(&weak_map_proto, "has", 1, |interp, this, args| {
            let o = interp.weak_map_object(this)?;
            let key = args.first().and_then(|v| v.as_object_cloned());
            let key = match key {
                Some(k) => k,
                None => return Ok(Value::Bool(false)),
            };
            let data = o.0.borrow();
            if let ObjectKind::WeakMap(m) = &data.kind {
                return Ok(Value::Bool(m.entries.iter().any(|(k, _)| k.ptr_eq(&key))));
            }
            Ok(Value::Bool(false))
        });
        self.define_method(&weak_map_proto, "delete", 1, |interp, this, args| {
            let o = interp.weak_map_object(this)?;
            let key = args.first().and_then(|v| v.as_object_cloned());
            let key = match key {
                Some(k) => k,
                None => return Ok(Value::Bool(false)),
            };
            let mut data = o.0.borrow_mut();
            if let ObjectKind::WeakMap(m) = &mut data.kind {
                let before = m.entries.len();
                m.entries.retain(|(k, _)| !k.ptr_eq(&key));
                return Ok(Value::Bool(m.entries.len() != before));
            }
            Ok(Value::Bool(false))
        });

        let weak_set_proto = self.realm.intrinsics.weak_set_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "WeakSet",
            0,
            |interp, _this, _args| interp.throw_type_error("Constructor WeakSet requires 'new'"),
            |interp, _args, new_target| {
                let p = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(p),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::WeakSet(WeakSetData {
                        entries: Vec::new(),
                    }),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(weak_set_proto.clone()));
        weak_set_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("WeakSet", Value::Object(ctor.clone()));
        self.realm.intrinsics.weak_set_ctor = Some(ctor);
        self.define_method(&weak_set_proto, "add", 1, |interp, this, args| {
            let o = interp.weak_set_object(this)?;
            let key = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(k) => k,
                None => return interp.throw_type_error("Invalid value used in weak set"),
            };
            let mut data = o.0.borrow_mut();
            if let ObjectKind::WeakSet(s) = &mut data.kind {
                if !s.entries.iter().any(|k| k.ptr_eq(&key)) {
                    s.entries.push(key);
                }
            }
            drop(data);
            Ok(Value::Object(o))
        });
        self.define_method(&weak_set_proto, "has", 1, |interp, this, args| {
            let o = interp.weak_set_object(this)?;
            let key = args.first().and_then(|v| v.as_object_cloned());
            let key = match key {
                Some(k) => k,
                None => return Ok(Value::Bool(false)),
            };
            let data = o.0.borrow();
            if let ObjectKind::WeakSet(s) = &data.kind {
                return Ok(Value::Bool(s.entries.iter().any(|k| k.ptr_eq(&key))));
            }
            Ok(Value::Bool(false))
        });
        self.define_method(&weak_set_proto, "delete", 1, |interp, this, args| {
            let o = interp.weak_set_object(this)?;
            let key = args.first().and_then(|v| v.as_object_cloned());
            let key = match key {
                Some(k) => k,
                None => return Ok(Value::Bool(false)),
            };
            let mut data = o.0.borrow_mut();
            if let ObjectKind::WeakSet(s) = &mut data.kind {
                let before = s.entries.len();
                s.entries.retain(|k| !k.ptr_eq(&key));
                return Ok(Value::Bool(s.entries.len() != before));
            }
            Ok(Value::Bool(false))
        });
    }

    fn map_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::Map(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("Method Map.prototype called on incompatible receiver")
    }
    fn set_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::Set(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("Method Set.prototype called on incompatible receiver")
    }
    fn weak_map_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::WeakMap(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("Method WeakMap.prototype called on incompatible receiver")
    }
    fn weak_set_object(&mut self, this: &Value) -> JsResult<JsObject> {
        if let Some(o) = this.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::WeakSet(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("Method WeakSet.prototype called on incompatible receiver")
    }

    fn map_entries(&mut self, map: &JsObject) -> Vec<(Value, Value)> {
        match &map.borrow().kind {
            ObjectKind::Map(m) => m.entries.clone(),
            _ => Vec::new(),
        }
    }
    fn map_get(&mut self, map: &JsObject, key: &Value) -> Value {
        match &map.borrow().kind {
            ObjectKind::Map(m) => m
                .entries
                .iter()
                .find(|(k, _)| Value::same_value_zero(k, key))
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Undefined),
            _ => Value::Undefined,
        }
    }
    fn map_has(&mut self, map: &JsObject, key: &Value) -> bool {
        match &map.borrow().kind {
            ObjectKind::Map(m) => m
                .entries
                .iter()
                .any(|(k, _)| Value::same_value_zero(k, key)),
            _ => false,
        }
    }
    fn map_set(
        &mut self,
        map: &JsObject,
        key: Value,
        value: Value,
        _normalize: bool,
    ) -> JsResult<()> {
        let mut data = map.0.borrow_mut();
        if let ObjectKind::Map(m) = &mut data.kind {
            if let Some(entry) = m
                .entries
                .iter_mut()
                .find(|(k, _)| Value::same_value_zero(k, &key))
            {
                entry.1 = value;
            } else {
                m.entries.push((key, value));
            }
        }
        Ok(())
    }
    fn map_delete(&mut self, map: &JsObject, key: &Value) -> bool {
        let mut data = map.0.borrow_mut();
        if let ObjectKind::Map(m) = &mut data.kind {
            let before = m.entries.len();
            m.entries.retain(|(k, _)| !Value::same_value_zero(k, key));
            return m.entries.len() != before;
        }
        false
    }
    fn set_entries(&mut self, set: &JsObject) -> Vec<Value> {
        match &set.borrow().kind {
            ObjectKind::Set(s) => s.entries.clone(),
            _ => Vec::new(),
        }
    }
    fn set_add(&mut self, set: &JsObject, value: Value) {
        if let ObjectKind::Set(s) = &mut set.0.borrow_mut().kind {
            if !s.entries.iter().any(|v| Value::same_value_zero(v, &value)) {
                s.entries.push(value);
            }
        }
    }
    fn set_has(&mut self, set: &JsObject, value: &Value) -> bool {
        match &set.borrow().kind {
            ObjectKind::Set(s) => s.entries.iter().any(|v| Value::same_value_zero(v, value)),
            _ => false,
        }
    }
    fn set_delete(&mut self, set: &JsObject, value: &Value) -> bool {
        if let ObjectKind::Set(s) = &mut set.0.borrow_mut().kind {
            let before = s.entries.len();
            s.entries.retain(|v| !Value::same_value_zero(v, value));
            return s.entries.len() != before;
        }
        false
    }
}

#[derive(Clone, Copy)]
enum CollectionIterKind {
    MapKey,
    MapValue,
    MapEntry,
    SetValue,
    SetEntry,
}

fn make_collection_iterator(
    interp: &mut Interp,
    target: &JsObject,
    kind: CollectionIterKind,
) -> JsResult<Value> {
    let index = Rc::new(RefCell::new(0usize));
    let target = target.clone();
    let next = native_fn("next", 0, move |interp, _this, _args| {
        let entries: Vec<(Value, Value)> = match &target.borrow().kind {
            ObjectKind::Map(m) => m.entries.clone(),
            ObjectKind::Set(s) => s.entries.iter().cloned().map(|v| (v.clone(), v)).collect(),
            _ => Vec::new(),
        };
        let mut i = index.borrow_mut();
        if *i >= entries.len() {
            let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
            obj.put_hidden("value", Value::Undefined);
            obj.put_hidden("done", Value::Bool(true));
            return Ok(Value::Object(obj));
        }
        let idx = *i;
        *i += 1;
        let (k, v) = entries[idx].clone();
        let value = match kind {
            CollectionIterKind::MapKey => k,
            CollectionIterKind::MapValue => v,
            CollectionIterKind::MapEntry => {
                Value::Object(interp.create_array_from_list(vec![k, v])?)
            }
            CollectionIterKind::SetValue => k,
            CollectionIterKind::SetEntry => {
                Value::Object(interp.create_array_from_list(vec![k.clone(), k])?)
            }
        };
        let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
        obj.put_hidden("value", value);
        obj.put_hidden("done", Value::Bool(false));
        Ok(Value::Object(obj))
    });
    next.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
    let proto = match kind {
        CollectionIterKind::MapKey
        | CollectionIterKind::MapValue
        | CollectionIterKind::MapEntry => interp.realm.intrinsics.map_iterator_prototype.clone(),
        _ => interp.realm.intrinsics.set_iterator_prototype.clone(),
    };
    let iter = JsObject::ordinary(Some(proto));
    iter.put_hidden("next", Value::Object(next));
    Ok(Value::Object(iter))
}
