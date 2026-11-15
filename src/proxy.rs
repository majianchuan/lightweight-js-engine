//! Proxy exotic object internal methods.

use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;

impl Interp {
    fn get_proxy_trap(&mut self, p: &ProxyData, name: &str) -> JsResult<Option<JsObject>> {
        let v = self.get_property(
            &Value::Object(p.handler.clone()),
            &PropertyKey::from_str(name),
        )?;
        match v.as_object_cloned() {
            Some(o) if o.is_callable() => Ok(Some(o)),
            _ => Ok(None),
        }
    }

    fn proxy_trap_this(&self, p: &ProxyData) -> Value {
        Value::Object(p.handler.clone())
    }

    pub fn proxy_get(
        &mut self,
        p: &ProxyData,
        key: &PropertyKey,
        receiver: &Value,
    ) -> JsResult<Value> {
        if let Some(trap) = self.get_proxy_trap(p, "get")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[
                    Value::Object(p.target.clone()),
                    key.to_value(),
                    receiver.clone(),
                ],
            )?;
            return Ok(result);
        }
        let target_desc = self.get_own_property(&p.target, key)?;
        if let Some(d) = &target_desc {
            if !d.configurable && d.value.is_accessor() {
                if let PropertyValue::Accessor { get: None, .. } = d.value {
                    return Ok(Value::Undefined);
                }
            }
        }
        self.get_property_with_receiver(&p.target, key, receiver)
    }

    pub fn proxy_set(
        &mut self,
        p: &ProxyData,
        key: &PropertyKey,
        value: Value,
        receiver: &Value,
    ) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "set")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[
                    Value::Object(p.target.clone()),
                    key.to_value(),
                    value,
                    receiver.clone(),
                ],
            )?;
            return Ok(result.truthy());
        }
        self.set_property_object(&p.target, key.clone(), value, receiver, false)
    }

    pub fn proxy_has(&mut self, p: &ProxyData, key: &PropertyKey) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "has")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone()), key.to_value()],
            )?;
            return Ok(result.truthy());
        }
        self.has_property(&p.target, key)
    }

    pub fn proxy_delete(&mut self, p: &ProxyData, key: &PropertyKey) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "deleteProperty")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone()), key.to_value()],
            )?;
            return Ok(result.truthy());
        }
        self.delete_property(&p.target, key)
    }

    pub fn proxy_own_keys(&mut self, p: &ProxyData) -> JsResult<Vec<PropertyKey>> {
        if let Some(trap) = self.get_proxy_trap(p, "ownKeys")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone())],
            )?;
            let obj = self.to_object(&result)?;
            let len = self.length_of_array_like(&obj)? as usize;
            let mut keys: Vec<PropertyKey> = Vec::with_capacity(len);
            let mut seen = std::collections::HashSet::new();
            for i in 0..len {
                let v = self.get_property(
                    &Value::Object(obj.clone()),
                    &PropertyKey::String(JsString::from_str(&i.to_string())),
                )?;
                let k = match v {
                    Value::String(s) => PropertyKey::String(s),
                    Value::Symbol(s) => PropertyKey::Symbol(s),
                    _ => return self.throw_type_error("Proxy ownKeys returned invalid key"),
                };
                if !seen.insert(k.clone()) {
                    return self.throw_type_error("Proxy ownKeys returned duplicate key");
                }
                keys.push(k);
            }
            return Ok(keys);
        }
        self.own_property_keys(&p.target)
    }

    pub fn proxy_get_own_property(
        &mut self,
        p: &ProxyData,
        key: &PropertyKey,
    ) -> JsResult<Option<Property>> {
        if let Some(trap) = self.get_proxy_trap(p, "getOwnPropertyDescriptor")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone()), key.to_value()],
            )?;
            if result.is_undefined() {
                return Ok(None);
            }
            let desc = self.to_property_descriptor(&result)?;
            let prop = desc.to_complete(None);
            return Ok(Some(prop));
        }
        self.get_own_property(&p.target, key)
    }

    pub fn proxy_define_own_property(
        &mut self,
        p: &ProxyData,
        key: PropertyKey,
        desc: Descriptor,
    ) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "defineProperty")? {
            let desc_obj = Self::descriptor_to_object(self, &desc)?;
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[
                    Value::Object(p.target.clone()),
                    key.to_value(),
                    Value::Object(desc_obj),
                ],
            )?;
            return Ok(result.truthy());
        }
        self.define_own_property(&p.target, key, desc)
    }

    fn descriptor_to_object(&mut self, desc: &Descriptor) -> JsResult<JsObject> {
        let obj = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        if let Some(v) = &desc.value {
            self.create_data_property(&obj, PropertyKey::from_str("value"), v.clone())?;
        }
        if let Some(w) = desc.writable {
            self.create_data_property(&obj, PropertyKey::from_str("writable"), Value::Bool(w))?;
        }
        if let Some(e) = desc.enumerable {
            self.create_data_property(&obj, PropertyKey::from_str("enumerable"), Value::Bool(e))?;
        }
        if let Some(c) = desc.configurable {
            self.create_data_property(&obj, PropertyKey::from_str("configurable"), Value::Bool(c))?;
        }
        if let Some(g) = &desc.get {
            self.create_data_property(
                &obj,
                PropertyKey::from_str("get"),
                g.clone().map(Value::Object).unwrap_or(Value::Undefined),
            )?;
        }
        if let Some(s) = &desc.set {
            self.create_data_property(
                &obj,
                PropertyKey::from_str("set"),
                s.clone().map(Value::Object).unwrap_or(Value::Undefined),
            )?;
        }
        Ok(obj)
    }

    pub fn proxy_get_prototype_of(&mut self, p: &ProxyData) -> JsResult<Option<JsObject>> {
        if let Some(trap) = self.get_proxy_trap(p, "getPrototypeOf")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone())],
            )?;
            return match result {
                Value::Null => Ok(None),
                Value::Object(o) => Ok(Some(o)),
                _ => self.throw_type_error("Proxy getPrototypeOf returned non-object"),
            };
        }
        self.get_prototype_of(&p.target)
    }

    pub fn proxy_set_prototype_of(
        &mut self,
        p: &ProxyData,
        proto: Option<JsObject>,
    ) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "setPrototypeOf")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[
                    Value::Object(p.target.clone()),
                    proto.clone().map(Value::Object).unwrap_or(Value::Null),
                ],
            )?;
            return Ok(result.truthy());
        }
        self.set_prototype_of(&p.target, proto)
    }

    pub fn proxy_is_extensible(&mut self, p: &ProxyData) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "isExtensible")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone())],
            )?;
            return Ok(result.truthy());
        }
        self.is_extensible(&p.target)
    }

    pub fn proxy_prevent_extensions(&mut self, p: &ProxyData) -> JsResult<bool> {
        if let Some(trap) = self.get_proxy_trap(p, "preventExtensions")? {
            let result = self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[Value::Object(p.target.clone())],
            )?;
            return Ok(result.truthy());
        }
        self.prevent_extensions(&p.target)
    }

    pub fn proxy_call(&mut self, p: &ProxyData, this: &Value, args: &[Value]) -> JsResult<Value> {
        if let Some(trap) = self.get_proxy_trap(p, "apply")? {
            let args_arr = self.create_array_from_list(args.to_vec())?;
            return self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[
                    Value::Object(p.target.clone()),
                    this.clone(),
                    Value::Object(args_arr),
                ],
            );
        }
        self.call_function(&Value::Object(p.target.clone()), this, args)
    }

    pub fn proxy_construct(
        &mut self,
        p: &ProxyData,
        args: &[Value],
        new_target: &JsObject,
    ) -> JsResult<Value> {
        if let Some(trap) = self.get_proxy_trap(p, "construct")? {
            let args_arr = self.create_array_from_list(args.to_vec())?;
            return self.call_function(
                &Value::Object(trap),
                &self.proxy_trap_this(p),
                &[
                    Value::Object(p.target.clone()),
                    Value::Object(args_arr),
                    Value::Object(new_target.clone()),
                ],
            );
        }
        self.construct_function(&Value::Object(p.target.clone()), args, Some(new_target))
    }
}
