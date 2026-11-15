//! Built-in objects and functions.

use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::rc::Rc;

pub fn native_fn(
    name: impl Into<JsString>,
    length: u32,
    f: impl Fn(&mut Interp, &Value, &[Value]) -> JsResult<Value> + 'static,
) -> JsObject {
    let name = name.into();
    let proto = JsObject::native_function(None, name, length, Rc::new(f), None);
    proto
}

pub fn native_ctor(
    name: impl Into<JsString>,
    length: u32,
    call: impl Fn(&mut Interp, &Value, &[Value]) -> JsResult<Value> + 'static,
    construct: impl Fn(&mut Interp, &[Value], &JsObject) -> JsResult<Value> + 'static,
) -> JsObject {
    let name = name.into();
    JsObject::native_function(None, name, length, Rc::new(call), Some(Rc::new(construct)))
}

impl Interp {
    pub fn init_core(&mut self) {
        let function_proto = self.realm.intrinsics.function_prototype.clone();
        // All native functions get Function.prototype after creation.
        let mut fix = |o: &JsObject| {
            o.set_prototype_raw(Some(function_proto.clone()));
        };

        self.install_object();
        self.install_function();
        self.install_array();
        self.install_string();
        self.install_array_methods();
        self.install_string_methods();
        self.install_regexp();
        self.install_json();
        self.install_collections();
        self.install_proxy();
        self.install_typed_arrays();
        self.install_iterator_helpers();
        self.install_async_generator();
        self.install_date();
        self.install_generator();
        self.install_async();
        self.install_promise();
        self.install_number();
        self.install_boolean();
        self.install_symbol();
        self.install_bigint();
        self.install_errors();
        self.install_math();
        self.install_misc_globals();

        // Fix prototypes of functions created before Function.prototype existed.
        let global = self.realm.global.clone();
        let mut todo = Vec::new();
        if let Some(o) = self.realm.intrinsics.object_ctor.clone() {
            todo.push(o);
        }
        if let Some(o) = self.realm.intrinsics.function_ctor.clone() {
            todo.push(o);
        }
        if let Some(o) = self.realm.intrinsics.array_ctor.clone() {
            todo.push(o);
        }
        for o in todo {
            fix(&o);
        }
    }

    fn define_global(&self, name: &str, value: Value) {
        self.realm.global.put_hidden(name, value);
    }

    fn install_object(&mut self) {
        let object_proto = self.realm.intrinsics.object_prototype.clone();
        let ctor = native_ctor(
            "Object",
            1,
            |interp, _this, args| {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                if value.is_nullish() {
                    let proto = interp.realm.intrinsics.object_prototype.clone();
                    Ok(Value::Object(JsObject::ordinary(Some(proto))))
                } else {
                    Ok(Value::Object(interp.to_object(&value)?))
                }
            },
            |interp, args, new_target| {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                if value.is_nullish() {
                    let proto = interp.prototype_from_new_target(new_target);
                    Ok(Value::Object(JsObject::ordinary(Some(proto))))
                } else {
                    Ok(Value::Object(interp.to_object(&value)?))
                }
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        object_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(object_proto.clone()));
        self.realm
            .global
            .put_hidden("Object", Value::Object(ctor.clone()));
        self.realm.intrinsics.object_ctor = Some(ctor.clone());

        let p = object_proto.clone();
        self.define_method(&p, "hasOwnProperty", 1, |interp, this, args| {
            let key = interp.to_property_key(args.first().unwrap_or(&Value::Undefined))?;
            let obj = interp.to_object(this)?;
            Ok(Value::Bool(interp.get_own_property(&obj, &key)?.is_some()))
        });
        self.define_method(&p, "isPrototypeOf", 1, |interp, this, args| {
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let mut obj = match value.as_object_cloned() {
                Some(o) => o,
                None => return Ok(Value::Bool(false)),
            };
            let this_obj = interp.to_object(this)?;
            loop {
                let proto = obj.0.borrow().prototype.clone();
                match proto {
                    Some(proto) => {
                        if proto.ptr_eq(&this_obj) {
                            return Ok(Value::Bool(true));
                        }
                        obj = proto;
                    }
                    None => return Ok(Value::Bool(false)),
                }
            }
        });
        self.define_method(&p, "propertyIsEnumerable", 1, |interp, this, args| {
            let key = interp.to_property_key(args.first().unwrap_or(&Value::Undefined))?;
            let obj = interp.to_object(this)?;
            Ok(Value::Bool(
                interp
                    .get_own_property(&obj, &key)?
                    .map(|p| p.enumerable)
                    .unwrap_or(false),
            ))
        });
        self.define_method(&p, "toString", 0, |interp, this, _args| {
            Ok(Value::String(interp.object_to_string_tag(this)?))
        });
        self.define_method(&p, "toLocaleString", 0, |interp, this, _args| {
            let method = interp.get_property(this, &PropertyKey::from_str("toString"))?;
            interp.call_function(&method, this, &[])
        });
        self.define_method(&p, "valueOf", 0, |interp, this, _args| {
            Ok(interp
                .to_object(this)
                .map(Value::Object)
                .unwrap_or(Value::Undefined))
        });
        // __proto__ accessor
        let getter = self.native_method("get __proto__", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let proto = obj.0.borrow().prototype.clone();
            Ok(proto.map(Value::Object).unwrap_or(Value::Null))
        });
        let setter = self.native_method("set __proto__", 1, |interp, this, args| {
            let obj = interp.to_object(this)?;
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            match v {
                Value::Object(p) => {
                    interp.set_prototype_of(&obj, Some(p))?;
                }
                Value::Null => {
                    interp.set_prototype_of(&obj, None)?;
                }
                _ => {}
            }
            Ok(Value::Undefined)
        });
        self.define_accessor(&p, "__proto__", getter, setter);

        self.define_method(&ctor, "getPrototypeOf", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            Ok(interp
                .get_prototype_of(&obj)?
                .map(Value::Object)
                .unwrap_or(Value::Null))
        });
        self.define_method(&ctor, "setPrototypeOf", 2, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let proto = args.get(1).cloned().unwrap_or(Value::Undefined);
            let proto = match proto {
                Value::Object(p) => Some(p),
                Value::Null => None,
                _ => {
                    return interp
                        .throw_type_error("Object prototype may only be an Object or null")
                }
            };
            if !interp.set_prototype_of(&obj, proto)? {
                return interp.throw_type_error("Cannot set prototype");
            }
            Ok(Value::Object(obj))
        });
        self.define_method(
            &ctor,
            "getOwnPropertyDescriptor",
            2,
            |interp, _this, args| {
                let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
                let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                match interp.get_own_property(&obj, &key)? {
                    Some(p) => Ok(Value::Object(interp.from_property_descriptor(&p)?)),
                    None => Ok(Value::Undefined),
                }
            },
        );
        self.define_method(
            &ctor,
            "getOwnPropertyDescriptors",
            1,
            |interp, _this, args| {
                let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
                let out =
                    JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
                let keys = interp.own_property_keys(&obj)?;
                for k in keys {
                    if let Some(p) = interp.get_own_property(&obj, &k)? {
                        let desc = interp.from_property_descriptor(&p)?;
                        interp.create_data_property(&out, k, Value::Object(desc))?;
                    }
                }
                Ok(Value::Object(out))
            },
        );
        self.define_method(&ctor, "getOwnPropertyNames", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let keys = interp.own_property_keys(&obj)?;
            let names: Vec<Value> = keys
                .into_iter()
                .filter_map(|k| k.as_string().map(|s| Value::String(s.clone())))
                .collect();
            Ok(Value::Object(interp.create_array_from_list(names)?))
        });
        self.define_method(&ctor, "getOwnPropertySymbols", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let keys = interp.own_property_keys(&obj)?;
            let syms: Vec<Value> = keys
                .into_iter()
                .filter_map(|k| match k {
                    PropertyKey::Symbol(s) => Some(Value::Symbol(s)),
                    _ => None,
                })
                .collect();
            Ok(Value::Object(interp.create_array_from_list(syms)?))
        });
        self.define_method(&ctor, "defineProperty", 3, |interp, _this, args| {
            let obj = match args.first().cloned().unwrap_or(Value::Undefined) {
                Value::Object(o) => o,
                _ => return interp.throw_type_error("Object.defineProperty called on non-object"),
            };
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            let desc = interp.to_property_descriptor(args.get(2).unwrap_or(&Value::Undefined))?;
            if !interp.define_own_property(&obj, key, desc)? {
                return interp.throw_type_error("Cannot define property");
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "defineProperties", 2, |interp, _this, args| {
            let obj = match args.first().cloned().unwrap_or(Value::Undefined) {
                Value::Object(o) => o,
                _ => {
                    return interp.throw_type_error("Object.defineProperties called on non-object")
                }
            };
            let props = interp.to_object(args.get(1).unwrap_or(&Value::Undefined))?;
            let keys = interp.own_property_keys(&props)?;
            for k in keys {
                if let Some(p) = interp.get_own_property(&props, &k)? {
                    if p.enumerable {
                        let v = interp.get_property(&Value::Object(props.clone()), &k)?;
                        let desc = interp.to_property_descriptor(&v)?;
                        if !interp.define_own_property(&obj, k, desc)? {
                            return interp.throw_type_error("Cannot define property");
                        }
                    }
                }
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "create", 2, |interp, _this, args| {
            let proto = match args.first().cloned().unwrap_or(Value::Undefined) {
                Value::Object(o) => Some(o),
                Value::Null => None,
                _ => {
                    return interp
                        .throw_type_error("Object prototype may only be an Object or null")
                }
            };
            let obj = JsObject::ordinary(proto);
            if let Some(props) = args.get(1) {
                if !props.is_undefined() {
                    let props = interp.to_object(props)?;
                    let keys = interp.own_property_keys(&props)?;
                    for k in keys {
                        if let Some(p) = interp.get_own_property(&props, &k)? {
                            if p.enumerable {
                                let v = interp.get_property(&Value::Object(props.clone()), &k)?;
                                let desc = interp.to_property_descriptor(&v)?;
                                if !interp.define_own_property(&obj, k, desc)? {
                                    return interp.throw_type_error("Cannot define property");
                                }
                            }
                        }
                    }
                }
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "keys", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let keys = interp.own_property_keys(&obj)?;
            let mut out = Vec::new();
            for k in keys {
                if let Some(p) = interp.get_own_property(&obj, &k)? {
                    if p.enumerable {
                        if let Some(s) = k.as_string() {
                            out.push(Value::String(s.clone()));
                        }
                    }
                }
            }
            Ok(Value::Object(interp.create_array_from_list(out)?))
        });
        self.define_method(&ctor, "values", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let keys = interp.own_property_keys(&obj)?;
            let mut out = Vec::new();
            for k in keys {
                if let Some(p) = interp.get_own_property(&obj, &k)? {
                    if p.enumerable {
                        if k.as_string().is_some() {
                            out.push(interp.get_property(&Value::Object(obj.clone()), &k)?);
                        }
                    }
                }
            }
            Ok(Value::Object(interp.create_array_from_list(out)?))
        });
        self.define_method(&ctor, "entries", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let keys = interp.own_property_keys(&obj)?;
            let mut out = Vec::new();
            for k in keys {
                if let Some(p) = interp.get_own_property(&obj, &k)? {
                    if p.enumerable {
                        if k.as_string().is_some() {
                            let v = interp.get_property(&Value::Object(obj.clone()), &k)?;
                            let pair = interp.create_array_from_list(vec![k.to_value(), v])?;
                            out.push(Value::Object(pair));
                        }
                    }
                }
            }
            Ok(Value::Object(interp.create_array_from_list(out)?))
        });
        self.define_method(&ctor, "assign", 2, |interp, _this, args| {
            let target = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            for src in args.iter().skip(1) {
                if src.is_nullish() {
                    continue;
                }
                let src = interp.to_object(src)?;
                let keys = interp.own_property_keys(&src)?;
                for k in keys {
                    if let Some(p) = interp.get_own_property(&src, &k)? {
                        if p.enumerable {
                            let v = interp.get_property(&Value::Object(src.clone()), &k)?;
                            interp.set_property_object(
                                &target,
                                k,
                                v,
                                &Value::Object(target.clone()),
                                true,
                            )?;
                        }
                    }
                }
            }
            Ok(Value::Object(target))
        });
        self.define_method(&ctor, "freeze", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            interp.prevent_extensions(&obj)?;
            let keys = interp.own_property_keys(&obj)?;
            for k in keys {
                if let Some(mut p) = interp.get_own_property(&obj, &k)? {
                    p.configurable = false;
                    if !p.value.is_accessor() {
                        p.writable = false;
                    }
                    interp.define_own_property(
                        &obj,
                        k,
                        Descriptor {
                            configurable: Some(false),
                            writable: if p.value.is_accessor() {
                                None
                            } else {
                                Some(false)
                            },
                            ..Default::default()
                        },
                    )?;
                }
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "seal", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            interp.prevent_extensions(&obj)?;
            let keys = interp.own_property_keys(&obj)?;
            for k in keys {
                interp.define_own_property(
                    &obj,
                    k,
                    Descriptor {
                        configurable: Some(false),
                        ..Default::default()
                    },
                )?;
            }
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "preventExtensions", 1, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            interp.prevent_extensions(&obj)?;
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "isExtensible", 1, |interp, _this, args| {
            match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => Ok(Value::Bool(interp.is_extensible(&o)?)),
                None => Ok(Value::Bool(false)),
            }
        });
        self.define_method(&ctor, "isFrozen", 1, |interp, _this, args| {
            match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => Ok(Value::Bool(interp.test_integrity_level(&o)?)),
                None => Ok(Value::Bool(true)),
            }
        });
        self.define_method(&ctor, "isSealed", 1, |interp, _this, args| {
            match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => {
                    let frozen = interp.test_integrity_level(&o)?;
                    let _ = frozen;
                    Ok(Value::Bool(
                        !interp.is_extensible(&o)? && interp.all_props_sealed(&o)?,
                    ))
                }
                None => Ok(Value::Bool(true)),
            }
        });
        self.define_method(&ctor, "is", 2, |_interp, _this, args| {
            Ok(Value::Bool(Value::same_value(
                args.first().unwrap_or(&Value::Undefined),
                args.get(1).unwrap_or(&Value::Undefined),
            )))
        });
        self.define_method(&ctor, "hasOwn", 2, |interp, _this, args| {
            let obj = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let key = interp.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(interp.get_own_property(&obj, &key)?.is_some()))
        });
    }

    fn install_function(&mut self) {
        let function_proto = self.realm.intrinsics.function_prototype.clone();
        function_proto.put_hidden("length", Value::Number(0.0));
        function_proto.put_hidden("name", Value::String(JsString::empty()));
        self.define_method(&function_proto, "call", 1, |interp, this, args| {
            let this_arg = args.first().cloned().unwrap_or(Value::Undefined);
            let rest = if args.len() > 1 { &args[1..] } else { &[] };
            interp.call_function(this, &this_arg, rest)
        });
        self.define_method(&function_proto, "apply", 2, |interp, this, args| {
            let this_arg = args.first().cloned().unwrap_or(Value::Undefined);
            let list = match args.get(1).cloned().unwrap_or(Value::Undefined) {
                Value::Undefined | Value::Null => Vec::new(),
                v => {
                    let obj = interp.to_object(&v)?;
                    let len = interp.length_of_array_like(&obj)? as usize;
                    let mut list = Vec::with_capacity(len);
                    for i in 0..len {
                        list.push(interp.get_property(
                            &Value::Object(obj.clone()),
                            &PropertyKey::String(JsString::from_str(&i.to_string())),
                        )?);
                    }
                    list
                }
            };
            interp.call_function(this, &this_arg, &list)
        });
        self.define_method(&function_proto, "bind", 1, |interp, this, args| {
            if !interp.is_callable_value(this) {
                return interp.throw_type_error("Bind must be called on a function");
            }
            let target = this.as_object_cloned().unwrap();
            let this_arg = args.first().cloned().unwrap_or(Value::Undefined);
            let bound_args = if args.len() > 1 {
                args[1..].to_vec()
            } else {
                Vec::new()
            };
            Ok(Value::Object(JsObject::bound_function(
                Some(interp.realm.intrinsics.function_prototype.clone()),
                target,
                this_arg,
                bound_args,
            )))
        });
        self.define_method(&function_proto, "toString", 0, |interp, this, _args| {
            let f = match this.as_object_cloned() {
                Some(f) if f.is_callable() => f,
                _ => {
                    return interp
                        .throw_type_error("Function.prototype.toString called on non-function")
                }
            };
            let text = {
                let data = f.0.borrow();
                match &data.kind {
                    ObjectKind::Function(FunctionKind::User(u)) => u
                        .proto
                        .source_text
                        .as_ref()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_else(|| format!("function {}() {{ [native code] }}", u.name)),
                    ObjectKind::Function(FunctionKind::Bound(_)) => {
                        "function () { [native code] }".to_string()
                    }
                    _ => {
                        let name = f
                            .function_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default();
                        format!("function {}() {{ [native code] }}", name)
                    }
                }
            };
            Ok(Value::String(JsString::from_str(&text)))
        });

        let ctor = native_ctor(
            "Function",
            1,
            |interp, _this, args| interp.create_dynamic_function(args, false),
            |interp, args, _nt| interp.create_dynamic_function(args, true),
        );
        ctor.set_prototype_raw(Some(function_proto.clone()));
        ctor.define_fixed("prototype", Value::Object(function_proto.clone()));
        self.realm
            .global
            .put_hidden("Function", Value::Object(ctor.clone()));
        self.realm.intrinsics.function_ctor = Some(ctor);
    }

    fn install_array(&mut self) {
        let array_proto = self.realm.intrinsics.array_prototype.clone();
        let ctor = native_ctor(
            "Array",
            1,
            |interp, _this, args| interp.construct_array(args, None),
            |interp, args, new_target| interp.construct_array(args, Some(new_target)),
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        array_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(array_proto.clone()));
        self.realm
            .global
            .put_hidden("Array", Value::Object(ctor.clone()));
        self.realm.intrinsics.array_ctor = Some(ctor.clone());
        self.define_method(&ctor, "isArray", 1, |interp, _this, args| {
            Ok(Value::Bool(
                interp.is_array(args.first().unwrap_or(&Value::Undefined)),
            ))
        });
    }

    fn install_string(&mut self) {
        let string_proto = self.realm.intrinsics.string_prototype.clone();
        let ctor = native_ctor(
            "String",
            1,
            |interp, _this, args| match args.first() {
                None => Ok(Value::String(JsString::empty())),
                Some(v) => Ok(Value::String(interp.to_string_allow_symbol(v)?)),
            },
            |interp, args, new_target| {
                let s = match args.first() {
                    None => JsString::empty(),
                    Some(v) => interp.to_string_allow_symbol(v)?,
                };
                let proto = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(proto),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::StringObject(s),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        string_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(string_proto.clone()));
        self.realm
            .global
            .put_hidden("String", Value::Object(ctor.clone()));
        self.realm.intrinsics.string_ctor = Some(ctor.clone());
        self.define_method(&string_proto, "toString", 0, |interp, this, _args| {
            interp.string_value_of(this)
        });
        self.define_method(&string_proto, "valueOf", 0, |interp, this, _args| {
            interp.string_value_of(this)
        });
        self.define_method(&ctor, "fromCharCode", 1, |interp, _this, args| {
            let mut units = Vec::with_capacity(args.len());
            for a in args {
                units.push(interp.to_uint16(a)?);
            }
            Ok(Value::String(JsString::from_utf16(units)))
        });
        self.define_method(&ctor, "fromCodePoint", 1, |interp, _this, args| {
            let mut units = Vec::new();
            for a in args {
                let n = interp.to_number(a)?;
                if !n.is_finite() || n < 0.0 || n > 0x10FFFF as f64 || n.fract() != 0.0 {
                    return interp.throw_range_error("Invalid code point");
                }
                let cp = n as u32;
                if cp <= 0xFFFF {
                    units.push(cp as u16);
                } else {
                    let v = cp - 0x10000;
                    units.push(0xD800 + (v >> 10) as u16);
                    units.push(0xDC00 + (v & 0x3FF) as u16);
                }
            }
            Ok(Value::String(JsString::from_utf16(units)))
        });
        self.define_method(&ctor, "raw", 1, |interp, _this, args| {
            let cooked = interp.to_object(args.first().unwrap_or(&Value::Undefined))?;
            let raw = interp.get_property(
                &Value::Object(cooked.clone()),
                &PropertyKey::from_str("raw"),
            )?;
            let raw = interp.to_object(&raw)?;
            let len = interp.length_of_array_like(&raw)? as usize;
            let mut out = JsString::empty();
            for i in 0..len {
                let seg = interp.get_property(
                    &Value::Object(raw.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                out = out.concat(&interp.to_string(&seg)?);
                if i + 1 < len {
                    if let Some(sub) = args.get(i + 1) {
                        out = out.concat(&interp.to_string(sub)?);
                    }
                }
            }
            Ok(Value::String(out))
        });
    }

    fn install_number(&mut self) {
        let number_proto = self.realm.intrinsics.number_prototype.clone();
        let ctor = native_ctor(
            "Number",
            1,
            |interp, _this, args| {
                Ok(Value::Number(match args.first() {
                    Some(v) => interp.to_number(v)?,
                    None => 0.0,
                }))
            },
            |interp, args, new_target| {
                let n = match args.first() {
                    Some(v) => interp.to_number(v)?,
                    None => 0.0,
                };
                let proto = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(proto),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::NumberObject(n),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        number_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(number_proto.clone()));
        self.realm
            .global
            .put_hidden("Number", Value::Object(ctor.clone()));
        self.realm.intrinsics.number_ctor = Some(ctor.clone());
        for (name, val) in [
            ("MAX_VALUE", f64::MAX),
            ("MIN_VALUE", f64::MIN_POSITIVE),
            ("NaN", f64::NAN),
            ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            ("POSITIVE_INFINITY", f64::INFINITY),
            ("MAX_SAFE_INTEGER", 9007199254740991.0),
            ("MIN_SAFE_INTEGER", -9007199254740991.0),
            ("EPSILON", f64::EPSILON),
        ] {
            ctor.define_fixed(name, Value::Number(val));
        }
        self.define_method(&number_proto, "valueOf", 0, |interp, this, _args| {
            interp.number_value_of(this)
        });
        self.define_method(&number_proto, "toString", 1, |interp, this, args| {
            let nv = interp.number_value_of(this)?;
            let n = interp.to_number(&nv)?;
            let radix = match args.first() {
                Some(v) if !v.is_undefined() => interp.to_int32(v)?,
                _ => 10,
            };
            if radix == 10 {
                return Ok(Value::String(JsString::from_str(&format_number(n))));
            }
            if !(2..=36).contains(&radix) {
                return interp.throw_range_error("toString() radix must be between 2 and 36");
            }
            Ok(Value::String(JsString::from_str(&number_to_radix(
                n,
                radix as u32,
            ))))
        });
        self.define_method(&number_proto, "toFixed", 1, |interp, this, args| {
            let nv = interp.number_value_of(this)?;
            let n = interp.to_number(&nv)?;
            let digits = match args.first() {
                Some(v) => interp.to_int32(v)?,
                None => 0,
            };
            if !(0..=100).contains(&digits) {
                return interp
                    .throw_range_error("toFixed() digits argument must be between 0 and 100");
            }
            Ok(Value::String(JsString::from_str(&format!(
                "{:.*}",
                digits as usize, n
            ))))
        });
        self.define_method(&number_proto, "toExponential", 1, |interp, this, args| {
            let nv = interp.number_value_of(this)?;
            let n = interp.to_number(&nv)?;
            let digits = match args.first() {
                Some(v) if !v.is_undefined() => Some(interp.to_int32(v)?),
                _ => None,
            };
            if let Some(d) = digits {
                if !(0..=100).contains(&d) {
                    return interp.throw_range_error("toExponential() argument out of range");
                }
            }
            Ok(Value::String(JsString::from_str(&format_exponential(
                n, digits,
            ))))
        });
        self.define_method(&number_proto, "toPrecision", 1, |interp, this, args| {
            let nv = interp.number_value_of(this)?;
            let n = interp.to_number(&nv)?;
            match args.first() {
                Some(v) if !v.is_undefined() => {
                    let p = interp.to_int32(v)?;
                    if !(1..=100).contains(&p) {
                        return interp.throw_range_error("toPrecision() argument out of range");
                    }
                    Ok(Value::String(JsString::from_str(&format!(
                        "{:.*}",
                        p as usize, n
                    ))))
                }
                _ => Ok(Value::String(JsString::from_str(&format_number(n)))),
            }
        });
        self.define_method(&ctor, "isNaN", 1, |interp, _this, args| {
            let n = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(n.is_nan()))
        });
        self.define_method(&ctor, "isFinite", 1, |interp, _this, args| {
            let n = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(n.is_finite()))
        });
        self.define_method(&ctor, "isInteger", 1, |_interp, _this, args| {
            let n = match args.first() {
                Some(Value::Number(n)) => *n,
                _ => return Ok(Value::Bool(false)),
            };
            Ok(Value::Bool(n.is_finite() && n.fract() == 0.0))
        });
        self.define_method(&ctor, "isSafeInteger", 1, |_interp, _this, args| {
            let n = match args.first() {
                Some(Value::Number(n)) => *n,
                _ => return Ok(Value::Bool(false)),
            };
            Ok(Value::Bool(
                n.is_finite() && n.fract() == 0.0 && n.abs() <= 9007199254740991.0,
            ))
        });
    }

    fn install_boolean(&mut self) {
        let boolean_proto = self.realm.intrinsics.boolean_prototype.clone();
        let ctor = native_ctor(
            "Boolean",
            1,
            |interp, _this, args| {
                Ok(Value::Bool(
                    interp.to_boolean(args.first().unwrap_or(&Value::Undefined)),
                ))
            },
            |interp, args, new_target| {
                let b = interp.to_boolean(args.first().unwrap_or(&Value::Undefined));
                let proto = interp.prototype_from_new_target(new_target);
                Ok(Value::Object(JsObject::new(ObjectData {
                    prototype: Some(proto),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::BooleanObject(b),
                    private: IndexMap::new(),
                })))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        boolean_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(boolean_proto.clone()));
        self.realm
            .global
            .put_hidden("Boolean", Value::Object(ctor.clone()));
        self.realm.intrinsics.boolean_ctor = Some(ctor);
        self.define_method(&boolean_proto, "valueOf", 0, |interp, this, _args| {
            interp.boolean_value_of(this)
        });
        self.define_method(&boolean_proto, "toString", 0, |interp, this, _args| {
            let b = interp.boolean_value_of(this)?;
            Ok(Value::String(JsString::from_str(if b.truthy() {
                "true"
            } else {
                "false"
            })))
        });
    }

    fn install_symbol(&mut self) {
        let symbol_proto = self.realm.intrinsics.symbol_prototype.clone();
        let ctor = native_ctor(
            "Symbol",
            0,
            |interp, _this, args| {
                let desc = match args.first() {
                    None => None,
                    Some(v) if v.is_undefined() => None,
                    Some(v) => Some(interp.to_string(v)?),
                };
                let sym = interp.realm.new_symbol(desc);
                Ok(Value::Symbol(sym))
            },
            |interp, _args, _nt| interp.throw_type_error("Symbol is not a constructor"),
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        symbol_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(symbol_proto.clone()));
        self.realm
            .global
            .put_hidden("Symbol", Value::Object(ctor.clone()));
        self.realm.intrinsics.symbol_ctor = Some(ctor.clone());
        for (name, sym) in [
            ("iterator", self.realm.intrinsics.symbol_iterator.clone()),
            (
                "asyncIterator",
                self.realm.intrinsics.symbol_async_iterator.clone(),
            ),
            (
                "hasInstance",
                self.realm.intrinsics.symbol_has_instance.clone(),
            ),
            (
                "isConcatSpreadable",
                self.realm.intrinsics.symbol_is_concat_spreadable.clone(),
            ),
            ("match", self.realm.intrinsics.symbol_match.clone()),
            ("matchAll", self.realm.intrinsics.symbol_match_all.clone()),
            ("replace", self.realm.intrinsics.symbol_replace.clone()),
            ("search", self.realm.intrinsics.symbol_search.clone()),
            ("species", self.realm.intrinsics.symbol_species.clone()),
            ("split", self.realm.intrinsics.symbol_split.clone()),
            (
                "toPrimitive",
                self.realm.intrinsics.symbol_to_primitive.clone(),
            ),
            (
                "toStringTag",
                self.realm.intrinsics.symbol_to_string_tag.clone(),
            ),
            (
                "unscopables",
                self.realm.intrinsics.symbol_unscopables.clone(),
            ),
        ] {
            ctor.define_fixed(name, Value::Symbol(sym));
        }
        self.define_method(&ctor, "for", 1, |interp, _this, args| {
            let key = interp.to_string(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Symbol(interp.realm.symbol_for(key)))
        });
        self.define_method(&ctor, "keyFor", 1, |interp, _this, args| {
            match args.first() {
                Some(Value::Symbol(sym)) => {
                    for (k, v) in &interp.realm.intrinsics.symbol_registry {
                        if v.id == sym.id {
                            return Ok(Value::String(k.clone()));
                        }
                    }
                    Ok(Value::Undefined)
                }
                _ => interp.throw_type_error("Symbol.keyFor requires a symbol"),
            }
        });
        self.define_method(&symbol_proto, "toString", 0, |interp, this, _args| {
            let sym = interp.symbol_value_of(this)?;
            Ok(Value::String(symbol_descriptive_string(&sym)))
        });
        self.define_method(&symbol_proto, "valueOf", 0, |interp, this, _args| {
            Ok(Value::Symbol(interp.symbol_value_of(this)?))
        });
    }

    fn install_bigint(&mut self) {
        let bigint_proto = self.realm.intrinsics.bigint_prototype.clone();
        let ctor = native_ctor(
            "BigInt",
            1,
            |interp, _this, args| {
                let v = interp.to_bigint(args.first().unwrap_or(&Value::Undefined))?;
                Ok(Value::BigInt(v))
            },
            |interp, _args, _nt| interp.throw_type_error("BigInt is not a constructor"),
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        bigint_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        ctor.define_fixed("prototype", Value::Object(bigint_proto.clone()));
        self.realm
            .global
            .put_hidden("BigInt", Value::Object(ctor.clone()));
        self.realm.intrinsics.bigint_ctor = Some(ctor.clone());
        self.define_method(&ctor, "asIntN", 2, |interp, _this, args| {
            let bits = interp.to_index(args.first().unwrap_or(&Value::Undefined))? as u64;
            let b = interp.to_bigint(args.get(1).unwrap_or(&Value::Undefined))?;
            if bits == 0 {
                return Ok(Value::BigInt(Rc::new(num_bigint::BigInt::from(0))));
            }
            Ok(Value::BigInt(Rc::new(as_int_n(&b, bits))))
        });
        self.define_method(&ctor, "asUintN", 2, |interp, _this, args| {
            let bits = interp.to_index(args.first().unwrap_or(&Value::Undefined))? as u64;
            let b = interp.to_bigint(args.get(1).unwrap_or(&Value::Undefined))?;
            Ok(Value::BigInt(Rc::new(as_uint_n(&b, bits))))
        });
        self.define_method(&bigint_proto, "toString", 1, |interp, this, args| {
            let b = interp.bigint_value_of(this)?;
            let radix = match args.first() {
                Some(v) if !v.is_undefined() => interp.to_int32(v)?,
                _ => 10,
            };
            if !(2..=36).contains(&radix) {
                return interp.throw_range_error("toString() radix must be between 2 and 36");
            }
            Ok(Value::String(JsString::from_str(&bigint_to_radix(
                &b,
                radix as u32,
            ))))
        });
        self.define_method(&bigint_proto, "valueOf", 0, |interp, this, _args| {
            Ok(Value::BigInt(interp.bigint_value_of(this)?))
        });
        self.define_method(&bigint_proto, "toLocaleString", 0, |interp, this, _args| {
            let b = interp.bigint_value_of(this)?;
            Ok(Value::String(JsString::from_str(&b.to_string())))
        });
    }

    fn install_errors(&mut self) {
        let error_pairs: Vec<(&str, JsObject)> = vec![
            ("Error", self.realm.intrinsics.error_prototype.clone()),
            (
                "TypeError",
                self.realm.intrinsics.type_error_prototype.clone(),
            ),
            (
                "RangeError",
                self.realm.intrinsics.range_error_prototype.clone(),
            ),
            (
                "ReferenceError",
                self.realm.intrinsics.reference_error_prototype.clone(),
            ),
            (
                "SyntaxError",
                self.realm.intrinsics.syntax_error_prototype.clone(),
            ),
            (
                "EvalError",
                self.realm.intrinsics.eval_error_prototype.clone(),
            ),
            (
                "URIError",
                self.realm.intrinsics.uri_error_prototype.clone(),
            ),
            (
                "AggregateError",
                self.realm.intrinsics.aggregate_error_prototype.clone(),
            ),
            (
                "SuppressedError",
                self.realm.intrinsics.suppressed_error_prototype.clone(),
            ),
        ];
        for (name, proto) in error_pairs {
            let name_j = JsString::from_str(name);
            let name2 = name_j.clone();
            let proto_for_construct = proto.clone();
            let proto_for_call = proto.clone();
            let ctor = native_ctor(
                name_j,
                1,
                move |interp, _this, args| interp.create_error(&name2, &proto_for_call, args),
                {
                    let name3 = JsString::from_str(name);
                    move |interp, args, new_target| {
                        let p = interp.prototype_from_new_target(new_target);
                        let _ = &proto_for_construct;
                        interp.create_error(&name3, &p, args)
                    }
                },
            );
            ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
            proto.put_hidden("constructor", Value::Object(ctor.clone()));
            ctor.define_fixed("prototype", Value::Object(proto.clone()));
            self.realm
                .global
                .put_hidden(name, Value::Object(ctor.clone()));
            match name {
                "Error" => self.realm.intrinsics.error_ctor = Some(ctor),
                "TypeError" => self.realm.intrinsics.type_error_ctor = Some(ctor),
                "RangeError" => self.realm.intrinsics.range_error_ctor = Some(ctor),
                "ReferenceError" => self.realm.intrinsics.reference_error_ctor = Some(ctor),
                "SyntaxError" => self.realm.intrinsics.syntax_error_ctor = Some(ctor),
                "EvalError" => self.realm.intrinsics.eval_error_ctor = Some(ctor),
                "URIError" => self.realm.intrinsics.uri_error_ctor = Some(ctor),
                "AggregateError" => self.realm.intrinsics.aggregate_error_ctor = Some(ctor),
                "SuppressedError" => self.realm.intrinsics.suppressed_error_ctor = Some(ctor),
                _ => {}
            }
        }
        let p = self.realm.intrinsics.error_prototype.clone();
        self.define_method(&p, "toString", 0, |interp, this, _args| {
            let obj = interp.to_object(this)?;
            let name = interp.get_property_key(&Value::Object(obj.clone()), "name")?;
            let name = if name.is_undefined() {
                JsString::from_str("Error")
            } else {
                interp.to_string(&name)?
            };
            let msg = interp.get_property_key(&Value::Object(obj.clone()), "message")?;
            let msg = if msg.is_undefined() {
                JsString::empty()
            } else {
                interp.to_string(&msg)?
            };
            if name.is_empty() {
                Ok(Value::String(msg))
            } else if msg.is_empty() {
                Ok(Value::String(name))
            } else {
                Ok(Value::String(
                    name.concat(&JsString::from_str(": ")).concat(&msg),
                ))
            }
        });
    }

    fn install_math(&mut self) {
        let math = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        for (name, val) in [
            ("E", std::f64::consts::E),
            ("LN10", std::f64::consts::LN_10),
            ("LN2", std::f64::consts::LN_2),
            ("LOG10E", std::f64::consts::LOG10_E),
            ("LOG2E", std::f64::consts::LOG2_E),
            ("PI", std::f64::consts::PI),
            ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
            ("SQRT2", std::f64::consts::SQRT_2),
        ] {
            math.define_fixed(name, Value::Number(val));
        }
        math.define_fixed(
            "Symbol.toStringTag",
            Value::String(JsString::from_str("Math")),
        );
        let unary = [
            ("abs", (|x: f64| x.abs()) as fn(f64) -> f64),
            ("acos", f64::acos),
            ("acosh", f64::acosh),
            ("asin", f64::asin),
            ("asinh", f64::asinh),
            ("atan", f64::atan),
            ("atanh", f64::atanh),
            ("cbrt", f64::cbrt),
            ("ceil", f64::ceil),
            ("cos", f64::cos),
            ("cosh", f64::cosh),
            ("exp", f64::exp),
            ("expm1", f64::exp_m1),
            ("floor", f64::floor),
            ("fround", |x: f64| x as f32 as f64),
            ("log", f64::ln),
            ("log10", f64::log10),
            ("log1p", f64::ln_1p),
            ("log2", f64::log2),
            ("round", js_round),
            ("sign", js_sign),
            ("sin", f64::sin),
            ("sinh", f64::sinh),
            ("sqrt", f64::sqrt),
            ("tan", f64::tan),
            ("tanh", f64::tanh),
            ("trunc", f64::trunc),
        ];
        for (name, f) in unary {
            let func = native_fn(format!("Math.{}", name), 1, move |interp, _this, args| {
                let x = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
                Ok(Value::Number(f(x)))
            });
            func.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
            math.put_hidden(name, Value::Object(func));
        }
        let binary = [
            (
                "atan2",
                (|a: f64, b: f64| a.atan2(b)) as fn(f64, f64) -> f64,
            ),
            ("pow", f64::powf),
            ("hypot", |a: f64, b: f64| a.hypot(b)),
            ("max", f64::max),
            ("min", f64::min),
        ];
        for (name, f) in binary {
            let func = native_fn(format!("Math.{}", name), 2, move |interp, _this, args| {
                if name == "max" || name == "min" {
                    if args.is_empty() {
                        return Ok(Value::Number(if name == "max" {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        }));
                    }
                    let mut nums = Vec::new();
                    for a in args {
                        let n = interp.to_number(a)?;
                        if n.is_nan() {
                            return Ok(Value::Number(f64::NAN));
                        }
                        nums.push(n);
                    }
                    return Ok(Value::Number(nums.into_iter().fold(
                        if name == "max" {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        },
                        |a, b| f(a, b),
                    )));
                }
                let a = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
                let b = interp.to_number(args.get(1).unwrap_or(&Value::Undefined))?;
                Ok(Value::Number(f(a, b)))
            });
            func.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
            math.put_hidden(name, Value::Object(func));
        }
        self.realm.global.put_hidden("Math", Value::Object(math));
    }

    fn install_misc_globals(&mut self) {
        let print = native_fn("print", 1, |interp, _this, args| {
            let mut parts = Vec::new();
            for a in args {
                parts.push(interp.to_string_allow_symbol(a)?.to_string_lossy());
            }
            println!("{}", parts.join(" "));
            Ok(Value::Undefined)
        });
        print.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        self.realm.global.put_hidden("print", Value::Object(print));

        let console = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        let log = native_fn("log", 1, |interp, _this, args| {
            let mut parts = Vec::new();
            for a in args {
                parts.push(interp.to_string_allow_symbol(a)?.to_string_lossy());
            }
            println!("{}", parts.join(" "));
            Ok(Value::Undefined)
        });
        log.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        console.put_hidden("log", Value::Object(log));
        self.realm
            .global
            .put_hidden("console", Value::Object(console));

        let global_this = self.global_this();
        self.realm.global.put_hidden("globalThis", global_this);
        let nan = native_fn("isNaN", 1, |interp, _this, args| {
            let n = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(n.is_nan()))
        });
        nan.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        self.realm.global.put_hidden("isNaN", Value::Object(nan));
        let finite = native_fn("isFinite", 1, |interp, _this, args| {
            let n = interp.to_number(args.first().unwrap_or(&Value::Undefined))?;
            Ok(Value::Bool(n.is_finite()))
        });
        finite.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        self.realm
            .global
            .put_hidden("isFinite", Value::Object(finite));

        let eval_fn = native_fn("eval", 1, |interp, _this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            let source = match &v {
                Value::String(s) => s.clone(),
                _ => return Ok(v),
            };
            interp.eval_source(&source)
        });
        eval_fn.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        self.realm.global.put_hidden("eval", Value::Object(eval_fn));

        let host = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        host.put_hidden("global", Value::Object(self.realm.global.clone()));
        let detach = native_fn("detachArrayBuffer", 1, |interp, _this, args| {
            let ab = match args.first().and_then(|v| v.as_object_cloned()) {
                Some(o) => o,
                None => {
                    return interp.throw_type_error("detachArrayBuffer requires an ArrayBuffer")
                }
            };
            if let ObjectKind::ArrayBuffer(a) = &mut ab.0.borrow_mut().kind {
                a.detached = true;
                a.bytes.borrow_mut().clear();
                return Ok(Value::Undefined);
            }
            interp.throw_type_error("detachArrayBuffer requires an ArrayBuffer")
        });
        detach.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        host.put_hidden("detachArrayBuffer", Value::Object(detach));
        let gc = native_fn("gc", 0, |_interp, _this, _args| Ok(Value::Undefined));
        gc.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        host.put_hidden("gc", Value::Object(gc));
        let eval_script = native_fn("evalScript", 1, |interp, _this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            let s = interp.to_string(&v)?;
            interp.eval_source(&s)
        });
        eval_script.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        host.put_hidden("evalScript", Value::Object(eval_script));
        self.realm.global.put_hidden("$262", Value::Object(host));
    }

    pub fn eval_source(&mut self, source: &JsString) -> JsResult<Value> {
        let text = source.to_string_lossy();
        let allocator = oxc_allocator::Allocator::default();
        let ret =
            oxc_parser::Parser::new(&allocator, &text, oxc_span::SourceType::default()).parse();
        if !ret.diagnostics.is_empty() {
            let msg = format!("{:?}", ret.diagnostics.first());
            return self.throw_syntax_error(&msg);
        }
        let proto = match crate::compiler::compile_eval(&text, &ret.program) {
            Ok(p) => p,
            Err(e) => return self.throw_syntax_error(&e.message),
        };
        self.run_script(proto)
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------
    pub fn define_method(
        &mut self,
        obj: &JsObject,
        name: &str,
        length: u32,
        f: impl Fn(&mut Interp, &Value, &[Value]) -> JsResult<Value> + 'static,
    ) {
        let func = native_fn(name, length, f);
        func.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        let desc = Descriptor {
            value: Some(Value::Object(func)),
            writable: Some(true),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(obj, PropertyKey::from_str(name), desc);
    }

    pub fn native_method(
        &mut self,
        name: &str,
        length: u32,
        f: impl Fn(&mut Interp, &Value, &[Value]) -> JsResult<Value> + 'static,
    ) -> JsObject {
        let func = native_fn(name, length, f);
        func.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        func
    }

    fn define_accessor(&mut self, obj: &JsObject, name: &str, get: JsObject, set: JsObject) {
        let desc = Descriptor {
            get: Some(Some(get)),
            set: Some(Some(set)),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(obj, PropertyKey::from_str(name), desc);
    }

    pub fn prototype_from_new_target(&mut self, new_target: &JsObject) -> JsObject {
        let proto = self
            .get_property(
                &Value::Object(new_target.clone()),
                &PropertyKey::from_str("prototype"),
            )
            .ok()
            .and_then(|v| v.as_object_cloned());
        proto.unwrap_or_else(|| self.realm.intrinsics.object_prototype.clone())
    }

    fn construct_array(
        &mut self,
        args: &[Value],
        _new_target: Option<&JsObject>,
    ) -> JsResult<Value> {
        let arr = JsObject::array(Some(self.realm.intrinsics.array_prototype.clone()));
        if args.len() == 1 {
            if let Value::Number(n) = args[0] {
                let len = self.to_uint32(&Value::Number(n))?;
                if len as f64 != n {
                    return self.throw_range_error("Invalid array length");
                }
                if let ObjectKind::Array(a) = &mut arr.0.borrow_mut().kind {
                    a.length = len;
                }
                return Ok(Value::Object(arr));
            }
        }
        for (i, v) in args.iter().enumerate() {
            arr.put(
                PropertyKey::String(JsString::from_str(&i.to_string())),
                v.clone(),
            );
        }
        if let ObjectKind::Array(a) = &mut arr.0.borrow_mut().kind {
            a.length = args.len() as u32;
        }
        Ok(Value::Object(arr))
    }

    fn create_error(
        &mut self,
        name: &JsString,
        proto: &JsObject,
        args: &[Value],
    ) -> JsResult<Value> {
        let obj = JsObject::ordinary(Some(proto.clone()));
        let msg = args.first().cloned().unwrap_or(Value::Undefined);
        if !msg.is_undefined() {
            let s = self.to_string(&msg)?;
            obj.put_hidden("message", Value::String(s));
        }
        if name.to_string_lossy() == "AggregateError" {
            let errors = args.get(1).cloned().unwrap_or(Value::Undefined);
            let list = if errors.is_nullish() {
                Vec::new()
            } else {
                self.iterable_to_list(&errors)?
            };
            obj.put("errors", Value::Object(self.create_array_from_list(list)?));
        }
        if name.to_string_lossy() == "SuppressedError" {
            if let Some(e) = args.first() {
                obj.put("error", e.clone());
            }
            if let Some(s) = args.get(1) {
                obj.put("suppressed", s.clone());
            }
        }
        Ok(Value::Object(obj))
    }

    fn string_value_of(&mut self, this: &Value) -> JsResult<Value> {
        match this {
            Value::String(s) => Ok(Value::String(s.clone())),
            Value::Object(o) => match &o.borrow().kind {
                ObjectKind::StringObject(s) => Ok(Value::String(s.clone())),
                _ => self
                    .throw_type_error("String.prototype.valueOf called on incompatible receiver"),
            },
            _ => self.throw_type_error("String.prototype.valueOf called on incompatible receiver"),
        }
    }

    fn number_value_of(&mut self, this: &Value) -> JsResult<Value> {
        match this {
            Value::Number(n) => Ok(Value::Number(*n)),
            Value::Object(o) => match &o.borrow().kind {
                ObjectKind::NumberObject(n) => Ok(Value::Number(*n)),
                _ => self
                    .throw_type_error("Number.prototype.valueOf called on incompatible receiver"),
            },
            _ => self.throw_type_error("Number.prototype.valueOf called on incompatible receiver"),
        }
    }

    fn boolean_value_of(&mut self, this: &Value) -> JsResult<Value> {
        match this {
            Value::Bool(b) => Ok(Value::Bool(*b)),
            Value::Object(o) => match &o.borrow().kind {
                ObjectKind::BooleanObject(b) => Ok(Value::Bool(*b)),
                _ => self
                    .throw_type_error("Boolean.prototype.valueOf called on incompatible receiver"),
            },
            _ => self.throw_type_error("Boolean.prototype.valueOf called on incompatible receiver"),
        }
    }

    fn symbol_value_of(&mut self, this: &Value) -> JsResult<SymbolValue> {
        match this {
            Value::Symbol(s) => Ok(s.clone()),
            Value::Object(o) => {
                let v = match &o.borrow().kind {
                    ObjectKind::SymbolObject(s) => s.clone(),
                    _ => {
                        return self.throw_type_error(
                            "Symbol.prototype.valueOf called on incompatible receiver",
                        )
                    }
                };
                Ok(v)
            }
            _ => self.throw_type_error("Symbol.prototype.valueOf called on incompatible receiver"),
        }
    }

    fn bigint_value_of(&mut self, this: &Value) -> JsResult<Rc<num_bigint::BigInt>> {
        match this {
            Value::BigInt(b) => Ok(b.clone()),
            Value::Object(o) => {
                let v = match &o.borrow().kind {
                    ObjectKind::BigIntObject(b) => b.clone(),
                    _ => {
                        return self.throw_type_error(
                            "BigInt.prototype.valueOf called on incompatible receiver",
                        )
                    }
                };
                Ok(v)
            }
            _ => self.throw_type_error("BigInt.prototype.valueOf called on incompatible receiver"),
        }
    }

    fn object_to_string_tag(&mut self, this: &Value) -> JsResult<JsString> {
        let (tag, obj) = match this {
            Value::Undefined => return Ok(JsString::from_str("[object Undefined]")),
            Value::Null => return Ok(JsString::from_str("[object Null]")),
            Value::Bool(_) => ("Boolean", None),
            Value::Number(_) => ("Number", None),
            Value::String(_) => ("String", None),
            Value::Symbol(_) => ("Symbol", None),
            Value::BigInt(_) => ("BigInt", None),
            Value::Object(o) => {
                let base = match &o.borrow().kind {
                    ObjectKind::Array(_) => "Array",
                    ObjectKind::Arguments(_) => "Arguments",
                    ObjectKind::Function(_) => "Function",
                    ObjectKind::Date(_) => "Date",
                    ObjectKind::RegExp(_) => "RegExp",
                    ObjectKind::Map(_) => "Map",
                    ObjectKind::Set(_) => "Set",
                    ObjectKind::WeakMap(_) => "WeakMap",
                    ObjectKind::WeakSet(_) => "WeakSet",
                    ObjectKind::Promise(_) => "Promise",
                    ObjectKind::ArrayBuffer(_) => "ArrayBuffer",
                    ObjectKind::TypedArray(t) => t.kind.name(),
                    ObjectKind::DataView(_) => "DataView",
                    ObjectKind::Proxy(_) => "Object",
                    _ => "Object",
                };
                (base, Some(o.clone()))
            }
        };
        if let Some(o) = obj {
            let tag_val = self.get_property(
                &Value::Object(o),
                &PropertyKey::Symbol(self.realm.intrinsics.symbol_to_string_tag.clone()),
            )?;
            if let Value::String(s) = tag_val {
                return Ok(JsString::from_str("[object ")
                    .concat(&s)
                    .concat(&JsString::from_str("]")));
            }
        }
        Ok(JsString::from_str(&format!("[object {}]", tag)))
    }

    pub fn create_dynamic_function(&mut self, args: &[Value], _construct: bool) -> JsResult<Value> {
        let mut params = Vec::new();
        let body = if args.is_empty() {
            JsString::empty()
        } else {
            for a in &args[..args.len() - 1] {
                params.push(self.to_string(a)?);
            }
            self.to_string(&args[args.len() - 1])?
        };
        let param_text = params
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            "function anonymous({}\n) {{\n{}\n}}",
            param_text,
            body.to_string_lossy()
        );
        let allocator = oxc_allocator::Allocator::default();
        let ret =
            oxc_parser::Parser::new(&allocator, &source, oxc_span::SourceType::default()).parse();
        if !ret.diagnostics.is_empty() {
            let msg = format!("{:?}", ret.diagnostics.first());
            return self.throw_syntax_error(&msg);
        }
        let proto = match crate::compiler::compile_script(&source, &ret.program) {
            Ok(p) => p,
            Err(e) => return self.throw_syntax_error(&e.message),
        };
        self.run_script(proto)?;
        let key = PropertyKey::from_str("anonymous");
        let f = self.get_property(&self.global_this(), &key)?;
        let _ = self.delete_property(&self.realm.global.clone(), &key);
        Ok(f)
    }

    pub fn all_props_sealed(&mut self, obj: &JsObject) -> JsResult<bool> {
        let keys = self.own_property_keys(obj)?;
        for k in keys {
            if let Some(p) = self.get_own_property(obj, &k)? {
                if p.configurable {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

fn symbol_descriptive_string(s: &SymbolValue) -> JsString {
    match &s.description {
        Some(d) => JsString::from_str("Symbol(")
            .concat(d)
            .concat(&JsString::from_str(")")),
        None => JsString::from_str("Symbol()"),
    }
}

fn number_to_radix(n: f64, radix: u32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    let neg = n < 0.0;
    let mut x = n.abs().trunc();
    let mut digits = Vec::new();
    while x >= 1.0 {
        let d = (x % radix as f64) as u32;
        digits.push(std::char::from_digit(d, radix).unwrap());
        x = (x / radix as f64).floor();
    }
    if digits.is_empty() {
        digits.push('0');
    }
    if neg {
        digits.push('-');
    }
    digits.reverse();
    digits.into_iter().collect()
}

fn bigint_to_radix(b: &num_bigint::BigInt, radix: u32) -> String {
    b.to_str_radix(radix)
}

fn as_uint_n(b: &num_bigint::BigInt, bits: u64) -> num_bigint::BigInt {
    use num_bigint::BigInt;
    let modulus = BigInt::from(1) << bits;
    let mut r = b % &modulus;
    if r.sign() == num_bigint::Sign::Minus {
        r += &modulus;
    }
    r
}

fn as_int_n(b: &num_bigint::BigInt, bits: u64) -> num_bigint::BigInt {
    use num_bigint::BigInt;
    let modulus = BigInt::from(1) << bits;
    let half = BigInt::from(1) << (bits - 1);
    let mut r = b % &modulus;
    if r.sign() == num_bigint::Sign::Minus {
        r += &modulus;
    }
    if r >= half {
        r -= modulus;
    }
    r
}

fn js_round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let f = x.floor();
    if x - f >= 0.5 {
        f + 1.0
    } else {
        f
    }
}

fn js_sign(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        x
    } else if x > 0.0 {
        1.0
    } else {
        -1.0
    }
}

fn format_exponential(n: f64, digits: Option<i32>) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    match digits {
        Some(d) => format!("{:.*e}", d as usize, n),
        None => format!("{:e}", n),
    }
}
