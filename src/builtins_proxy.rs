//! Proxy and Reflect built-ins.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::cell::Cell;
use std::rc::Rc;

impl Interp {
    pub fn install_proxy(&mut self) {
        let ctor = crate::builtins::native_ctor(
            "Proxy",
            2,
            |interp, _this, _args| interp.throw_type_error("Constructor Proxy requires 'new'"),
            |interp, args, _new_target| {
                let target = match args.first().and_then(|v| v.as_object_cloned()) {
                    Some(o) => o,
                    None => {
                        return interp
                            .throw_type_error("Cannot create proxy with a non-object as target")
                    }
                };
                let handler = match args.get(1).and_then(|v| v.as_object_cloned()) {
                    Some(o) => o,
                    None => {
                        return interp
                            .throw_type_error("Cannot create proxy with a non-object as handler")
                    }
                };
                let callable = target.is_callable();
                let constructible = target.is_constructor();
                let proxy = JsObject::new(ObjectData {
                    prototype: None,
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::Proxy(ProxyData {
                        target,
                        handler,
                        callable,
                        constructible,
                        revoked: Rc::new(Cell::new(false)),
                    }),
                    private: IndexMap::new(),
                });
                Ok(Value::Object(proxy))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        self.realm
            .global
            .put_hidden("Proxy", Value::Object(ctor.clone()));
        self.define_method(&ctor, "revocable", 2, |interp, _this, args| {
            let target = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp
                        .throw_type_error("Cannot create proxy with a non-object as target")
                }
            };
            let handler = match args.get(1).and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp
                        .throw_type_error("Cannot create proxy with a non-object as handler")
                }
            };
            let revoked = Rc::new(Cell::new(false));
            let proxy = JsObject::new(ObjectData {
                prototype: None,
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::Proxy(ProxyData {
                    callable: target.is_callable(),
                    constructible: target.is_constructor(),
                    target,
                    handler,
                    revoked: revoked.clone(),
                }),
                private: IndexMap::new(),
            });
            let revoke = native_fn("revoke", 0, move |_interp, _this, _args| {
                revoked.set(true);
                Ok(Value::Undefined)
            });
            revoke.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
            let out = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
            out.put("proxy", Value::Object(proxy));
            out.put("revoke", Value::Object(revoke));
            Ok(Value::Object(out))
        });
        self.install_reflect();
    }

    fn install_reflect(&mut self) {
        let reflect = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        reflect.define_fixed(
            "Symbol.toStringTag",
            Value::String(JsString::from_str("Reflect")),
        );
        self.realm
            .global
            .put_hidden("Reflect", Value::Object(reflect.clone()));

        self.define_method(&reflect, "apply", 3, |interp, _this, args| {
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
            let list = args.get(2).cloned().unwrap_or(Value::Undefined);
            let obj = interp.to_object(&list)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let mut call_args = Vec::with_capacity(len);
            for i in 0..len {
                call_args.push(interp.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?);
            }
            interp.call_function(&target, &this_arg, &call_args)
        });
        self.define_method(&reflect, "construct", 2, |interp, _this, args| {
            let target = args.first().cloned().unwrap_or(Value::Undefined);
            let list = args.get(1).cloned().unwrap_or(Value::Undefined);
            let new_target = args.get(2).cloned().unwrap_or(Value::Undefined);
            let obj = interp.to_object(&list)?;
            let len = interp.length_of_array_like(&obj)? as usize;
            let mut call_args = Vec::with_capacity(len);
            for i in 0..len {
                call_args.push(interp.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?);
            }
            let nt = new_target.as_object_cloned();
            interp.construct_function(&target, &call_args, nt.as_ref())
        });
        self.define_method(&reflect, "defineProperty", 3, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp.throw_type_error("Reflect.defineProperty called on non-object")
                }
            };
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            let desc = interp.to_property_descriptor(args.get(2).unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(interp.define_own_property(&obj, key, desc)?))
        });
        self.define_method(&reflect, "deleteProperty", 2, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp.throw_type_error("Reflect.deleteProperty called on non-object")
                }
            };
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(interp.delete_property(&obj, &key)?))
        });
        self.define_method(&reflect, "get", 2, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => return interp.throw_type_error("Reflect.get called on non-object"),
            };
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            let receiver = args.get(2).cloned().unwrap_or(Value::Object(obj.clone()));
            let kind = obj.borrow().kind.clone();
            if let ObjectKind::Proxy(p) = kind {
                interp.proxy_get(&p, &key, &receiver)
            } else {
                interp.get_property_with_receiver(&obj, &key, &receiver)
            }
        });
        self.define_method(
            &reflect,
            "getOwnPropertyDescriptor",
            2,
            |interp, _this, args| {
                let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                    Some(o) => o,
                    None => {
                        return interp.throw_type_error(
                            "Reflect.getOwnPropertyDescriptor called on non-object",
                        )
                    }
                };
                let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                match interp.get_own_property(&obj, &key)? {
                    Some(p) => Ok(Value::Object(interp.from_property_descriptor(&p)?)),
                    None => Ok(Value::Undefined),
                }
            },
        );
        self.define_method(&reflect, "getPrototypeOf", 1, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp.throw_type_error("Reflect.getPrototypeOf called on non-object")
                }
            };
            Ok(interp
                .get_prototype_of(&obj)?
                .map(Value::Object)
                .unwrap_or(Value::Null))
        });
        self.define_method(&reflect, "has", 2, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => return interp.throw_type_error("Reflect.has called on non-object"),
            };
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(interp.has_property(&obj, &key)?))
        });
        self.define_method(&reflect, "isExtensible", 1, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp.throw_type_error("Reflect.isExtensible called on non-object")
                }
            };
            Ok(Value::Bool(interp.is_extensible(&obj)?))
        });
        self.define_method(&reflect, "ownKeys", 1, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => return interp.throw_type_error("Reflect.ownKeys called on non-object"),
            };
            let keys = interp.own_property_keys(&obj)?;
            let vals: Vec<Value> = keys.into_iter().map(|k| k.to_value()).collect();
            Ok(Value::Object(interp.create_array_from_list(vals)?))
        });
        self.define_method(&reflect, "preventExtensions", 1, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp
                        .throw_type_error("Reflect.preventExtensions called on non-object")
                }
            };
            Ok(Value::Bool(interp.prevent_extensions(&obj)?))
        });
        self.define_method(&reflect, "set", 3, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => return interp.throw_type_error("Reflect.set called on non-object"),
            };
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            let value = args.get(2).cloned().unwrap_or(Value::Undefined);
            let receiver = args.get(3).cloned().unwrap_or(Value::Object(obj.clone()));
            let kind = obj.borrow().kind.clone();
            let result = if let ObjectKind::Proxy(p) = kind {
                interp.proxy_set(&p, &key, value, &receiver)?
            } else {
                interp.set_property_object(&obj, key, value, &receiver, false)?
            };
            Ok(Value::Bool(result))
        });
        self.define_method(&reflect, "setPrototypeOf", 2, |interp, _this, args| {
            let obj = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp.throw_type_error("Reflect.setPrototypeOf called on non-object")
                }
            };
            let proto = match args.get(1).cloned().unwrap_or(Value::Undefined) {
                Value::Object(p) => Some(p),
                Value::Null => None,
                _ => {
                    return interp
                        .throw_type_error("Object prototype may only be an Object or null")
                }
            };
            Ok(Value::Bool(interp.set_prototype_of(&obj, proto)?))
        });
    }
}
