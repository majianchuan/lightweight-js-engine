//! Abstract operations from the ECMAScript specification, implemented on the
//! interpreter.

use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use std::rc::Rc;

impl Interp {
    // ------------------------------------------------------------------
    // Errors
    // ------------------------------------------------------------------
    pub fn throw_type_error<T>(&mut self, msg: &str) -> JsResult<T> {
        let err = self
            .realm
            .realm_error(&self.realm.intrinsics.type_error_prototype.clone(), msg);
        Err(Completion::Throw(err))
    }
    pub fn throw_range_error<T>(&mut self, msg: &str) -> JsResult<T> {
        let err = self
            .realm
            .realm_error(&self.realm.intrinsics.range_error_prototype.clone(), msg);
        Err(Completion::Throw(err))
    }
    pub fn throw_reference_error<T>(&mut self, msg: &str) -> JsResult<T> {
        let err = self.realm.realm_error(
            &self.realm.intrinsics.reference_error_prototype.clone(),
            msg,
        );
        Err(Completion::Throw(err))
    }
    pub fn throw_syntax_error<T>(&mut self, msg: &str) -> JsResult<T> {
        let err = self
            .realm
            .realm_error(&self.realm.intrinsics.syntax_error_prototype.clone(), msg);
        Err(Completion::Throw(err))
    }
    pub fn throw_uri_error<T>(&mut self, msg: &str) -> JsResult<T> {
        let err = self
            .realm
            .realm_error(&self.realm.intrinsics.uri_error_prototype.clone(), msg);
        Err(Completion::Throw(err))
    }
    pub fn throw_error<T>(&mut self, proto: JsObject, msg: &str) -> JsResult<T> {
        let err = self.realm.realm_error(&proto, msg);
        Err(Completion::Throw(err))
    }

    // ------------------------------------------------------------------
    // Type conversion
    // ------------------------------------------------------------------
    pub fn to_boolean(&self, value: &Value) -> bool {
        value.truthy()
    }

    pub fn to_primitive(&mut self, value: &Value, hint: &str) -> JsResult<Value> {
        let obj = match value.as_object_cloned() {
            Some(o) => o,
            None => return Ok(value.clone()),
        };
        let sym_key = PropertyKey::Symbol(self.realm.intrinsics.symbol_to_primitive.clone());
        let exotic = self.get_property(&Value::Object(obj.clone()), &sym_key)?;
        if !exotic.is_nullish() {
            if !exotic
                .as_object_cloned()
                .map(|o| o.is_callable())
                .unwrap_or(false)
            {
                return self.throw_type_error("Symbol.toPrimitive is not a function");
            }
            let result = self.call_function(
                &exotic,
                &Value::Object(obj.clone()),
                &[Value::String(JsString::from_str(hint))],
            )?;
            if !result.is_primitive() {
                return self.throw_type_error("Cannot convert object to primitive value");
            }
            return Ok(result);
        }
        let order: [&str; 2] = if hint == "string" {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in order {
            let method =
                self.get_property(&Value::Object(obj.clone()), &PropertyKey::from_str(name))?;
            if method
                .as_object_cloned()
                .map(|o| o.is_callable())
                .unwrap_or(false)
            {
                let result = self.call_function(&method, &Value::Object(obj.clone()), &[])?;
                if result.is_primitive() {
                    return Ok(result);
                }
            }
        }
        self.throw_type_error("Cannot convert object to primitive value")
    }

    pub fn to_number(&mut self, value: &Value) -> JsResult<f64> {
        match value {
            Value::Undefined => Ok(f64::NAN),
            Value::Null => Ok(0.0),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Number(n) => Ok(*n),
            Value::BigInt(_) => self.throw_type_error("Cannot convert a BigInt value to a number"),
            Value::String(s) => Ok(string_to_number(s)),
            Value::Symbol(_) => self.throw_type_error("Cannot convert a Symbol value to a number"),
            Value::Object(_) => {
                let prim = self.to_primitive(value, "number")?;
                self.to_number(&prim)
            }
        }
    }

    pub fn to_numeric(&mut self, value: &Value) -> JsResult<Value> {
        let prim = self.to_primitive(value, "number")?;
        if prim.is_bigint() {
            return Ok(prim);
        }
        Ok(Value::Number(self.to_number(&prim)?))
    }

    pub fn to_string(&mut self, value: &Value) -> JsResult<JsString> {
        match value {
            Value::Undefined => Ok(JsString::from_str("undefined")),
            Value::Null => Ok(JsString::from_str("null")),
            Value::Bool(b) => Ok(JsString::from_str(if *b { "true" } else { "false" })),
            Value::Number(n) => Ok(JsString::from_str(&format_number(*n))),
            Value::BigInt(b) => Ok(JsString::from_str(&b.to_string())),
            Value::String(s) => Ok(s.clone()),
            Value::Symbol(_) => self.throw_type_error("Cannot convert a Symbol value to a string"),
            Value::Object(_) => {
                let prim = self.to_primitive(value, "string")?;
                self.to_string(&prim)
            }
        }
    }

    /// ToString without the Symbol restriction (used by String()).
    pub fn to_string_allow_symbol(&mut self, value: &Value) -> JsResult<JsString> {
        if let Value::Symbol(s) = value {
            return Ok(match &s.description {
                Some(d) => {
                    let mut out = JsString::from_str("Symbol(");
                    out = out.concat(d);
                    out.concat(&JsString::from_str(")"))
                }
                None => JsString::from_str("Symbol()"),
            });
        }
        self.to_string(value)
    }

    pub fn to_object(&mut self, value: &Value) -> JsResult<JsObject> {
        match value {
            Value::Undefined | Value::Null => {
                self.throw_type_error("Cannot convert undefined or null to object")
            }
            Value::Object(o) => Ok(o.clone()),
            Value::String(s) => Ok(JsObject::new(ObjectData {
                prototype: Some(self.realm.intrinsics.string_prototype.clone()),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::StringObject(s.clone()),
                private: IndexMap::new(),
            })),
            Value::Number(n) => Ok(JsObject::new(ObjectData {
                prototype: Some(self.realm.intrinsics.number_prototype.clone()),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::NumberObject(*n),
                private: IndexMap::new(),
            })),
            Value::Bool(b) => Ok(JsObject::new(ObjectData {
                prototype: Some(self.realm.intrinsics.boolean_prototype.clone()),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::BooleanObject(*b),
                private: IndexMap::new(),
            })),
            Value::Symbol(s) => Ok(JsObject::new(ObjectData {
                prototype: Some(self.realm.intrinsics.symbol_prototype.clone()),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::SymbolObject(s.clone()),
                private: IndexMap::new(),
            })),
            Value::BigInt(b) => Ok(JsObject::new(ObjectData {
                prototype: Some(self.realm.intrinsics.bigint_prototype.clone()),
                extensible: true,
                properties: IndexMap::new(),
                kind: ObjectKind::BigIntObject(b.clone()),
                private: IndexMap::new(),
            })),
        }
    }

    pub fn to_property_key(&mut self, value: &Value) -> JsResult<PropertyKey> {
        let prim = self.to_primitive(value, "string")?;
        match prim {
            Value::Symbol(s) => Ok(PropertyKey::Symbol(s)),
            _ => Ok(PropertyKey::String(self.to_string(&prim)?)),
        }
    }

    pub fn to_integer_or_infinity(&mut self, value: &Value) -> JsResult<f64> {
        let n = self.to_number(value)?;
        if n.is_nan() {
            return Ok(0.0);
        }
        if n == 0.0 {
            return Ok(0.0);
        }
        if n.is_infinite() {
            return Ok(n);
        }
        Ok(n.trunc())
    }

    pub fn to_length(&mut self, value: &Value) -> JsResult<f64> {
        let n = self.to_integer_or_infinity(value)?;
        if n <= 0.0 {
            return Ok(0.0);
        }
        Ok(n.min(9007199254740991.0))
    }

    pub fn to_index(&mut self, value: &Value) -> JsResult<usize> {
        let n = self.to_integer_or_infinity(value)?;
        if n < 0.0 || n > 9007199254740991.0 {
            return self.throw_range_error("Invalid index");
        }
        Ok(n as usize)
    }

    pub fn to_int32(&mut self, value: &Value) -> JsResult<i32> {
        let n = self.to_number(value)?;
        Ok(f64_to_int32(n))
    }

    pub fn to_uint32(&mut self, value: &Value) -> JsResult<u32> {
        let n = self.to_number(value)?;
        Ok(f64_to_uint32(n))
    }

    pub fn to_uint16(&mut self, value: &Value) -> JsResult<u16> {
        let n = self.to_number(value)?;
        Ok(f64_to_uint32(n) as u16)
    }

    pub fn to_bigint(&mut self, value: &Value) -> JsResult<Rc<BigInt>> {
        match value {
            Value::BigInt(b) => Ok(b.clone()),
            Value::Bool(b) => Ok(Rc::new(BigInt::from(if *b { 1 } else { 0 }))),
            Value::String(s) => string_to_bigint(s)
                .map(Rc::new)
                .ok_or(())
                .or_else(|_| self.throw_syntax_error("Cannot convert string to BigInt")),
            Value::Number(_) => self.throw_type_error("Cannot convert a Number to a BigInt"),
            Value::Undefined | Value::Null => {
                self.throw_type_error("Cannot convert undefined or null to a BigInt")
            }
            Value::Symbol(_) => self.throw_type_error("Cannot convert a Symbol to a BigInt"),
            Value::Object(_) => {
                let prim = self.to_primitive(value, "number")?;
                self.to_bigint(&prim)
            }
        }
    }

    // ------------------------------------------------------------------
    // Object internal methods
    // ------------------------------------------------------------------
    pub fn is_array(&self, value: &Value) -> bool {
        let mut current = value.as_object_cloned();
        loop {
            match current {
                Some(o) => {
                    let kind = o.borrow().kind.clone();
                    match kind {
                        ObjectKind::Array(_) => return true,
                        ObjectKind::Proxy(p) => {
                            current = Some(p.target);
                        }
                        _ => return false,
                    }
                }
                None => return false,
            }
        }
    }

    pub fn get_own_property(
        &mut self,
        obj: &JsObject,
        key: &PropertyKey,
    ) -> JsResult<Option<Property>> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_get_own_property(&p, key);
        }
        let p = obj.0.borrow().get_own_property(key);
        Ok(p)
    }

    pub fn check_deadline(&mut self) -> JsResult<()> {
        if self.deadline.is_none() {
            return Ok(());
        }
        self.check_counter = self.check_counter.wrapping_add(1);
        if self.check_counter % 4096 == 0 {
            if let Some(deadline) = self.deadline {
                if std::time::Instant::now() > deadline {
                    return self.throw_range_error("Execution time limit exceeded");
                }
            }
        }
        Ok(())
    }

    pub fn get_property(&mut self, value: &Value, key: &PropertyKey) -> JsResult<Value> {
        self.check_deadline()?;
        let mut obj = match value {
            Value::Object(o) => {
                if let ObjectKind::Proxy(p) = o.borrow().kind.clone() {
                    return self.proxy_get(&p, key, value);
                }
                o.clone()
            }
            Value::String(s) => {
                if let PropertyKey::String(k) = key {
                    if let Some(idx) = k.as_array_index() {
                        if (idx as usize) < s.len() {
                            return Ok(Value::String(JsString::from_utf16(vec![
                                s.as_slice()[idx as usize],
                            ])));
                        }
                    }
                    if k.to_string_lossy() == "length" {
                        return Ok(Value::Number(s.len() as f64));
                    }
                }
                if key
                    .as_string()
                    .map(|s| s.to_string_lossy() == "length")
                    .unwrap_or(false)
                {
                    return Ok(Value::Number(s.len() as f64));
                }
                self.realm.intrinsics.string_prototype.clone()
            }
            Value::Number(_) => self.realm.intrinsics.number_prototype.clone(),
            Value::Bool(_) => self.realm.intrinsics.boolean_prototype.clone(),
            Value::Symbol(_) => self.realm.intrinsics.symbol_prototype.clone(),
            Value::BigInt(_) => self.realm.intrinsics.bigint_prototype.clone(),
            Value::Undefined | Value::Null => {
                return self.throw_type_error("Cannot read properties of null or undefined")
            }
        };
        let receiver = value.clone();
        loop {
            let own = {
                let data = obj.0.borrow();
                data.get_own_property(key)
            };
            if let Some(prop) = own {
                return match prop.value {
                    PropertyValue::Data(v) => Ok(v),
                    PropertyValue::Accessor { get, .. } => match get {
                        Some(g) => self.call_function(&Value::Object(g), &receiver, &[]),
                        None => Ok(Value::Undefined),
                    },
                };
            }
            let proto = obj.0.borrow().prototype.clone();
            match proto {
                Some(p) => obj = p,
                None => return Ok(Value::Undefined),
            }
        }
    }

    pub fn get_property_key(&mut self, obj: &Value, key: &str) -> JsResult<Value> {
        self.get_property(obj, &PropertyKey::from_str(key))
    }

    pub fn has_property(&mut self, obj: &JsObject, key: &PropertyKey) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_has(&p, key);
        }
        let mut cur = obj.clone();
        loop {
            if cur.0.borrow().get_own_property(key).is_some() {
                return Ok(true);
            }
            let proto = cur.0.borrow().prototype.clone();
            match proto {
                Some(p) => cur = p,
                None => return Ok(false),
            }
        }
    }

    pub fn has_property_value(&mut self, value: &Value, key: &PropertyKey) -> JsResult<bool> {
        if let Value::Object(o) = value {
            self.has_property(o, key)
        } else {
            let obj = self.to_object(value)?;
            self.has_property(&obj, key)
        }
    }

    pub fn create_data_property(
        &mut self,
        obj: &JsObject,
        key: PropertyKey,
        value: Value,
    ) -> JsResult<bool> {
        let desc = Descriptor {
            value: Some(value),
            writable: Some(true),
            enumerable: Some(true),
            configurable: Some(true),
            ..Default::default()
        };
        self.define_own_property(obj, key, desc)
    }

    pub fn create_data_property_or_throw(
        &mut self,
        obj: &JsObject,
        key: PropertyKey,
        value: Value,
    ) -> JsResult<()> {
        if !self.create_data_property(obj, key.clone(), value)? {
            return self.throw_type_error("Cannot create property");
        }
        Ok(())
    }

    pub fn define_own_property(
        &mut self,
        obj: &JsObject,
        key: PropertyKey,
        desc: Descriptor,
    ) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_define_own_property(&p, key, desc);
        }
        self.ordinary_define_own_property(obj, key, desc)
    }

    fn ordinary_define_own_property(
        &mut self,
        obj: &JsObject,
        key: PropertyKey,
        desc: Descriptor,
    ) -> JsResult<bool> {
        // Array exotic behavior.
        let is_array = matches!(&obj.borrow().kind, ObjectKind::Array(_));
        if is_array {
            if let Some(idx) = key.as_array_index() {
                let len = match &obj.borrow().kind {
                    ObjectKind::Array(a) => a.length,
                    _ => 0,
                };
                if idx >= len {
                    let new_len = idx as u64 + 1;
                    if new_len > 0xFFFF_FFFF {
                        return Ok(false);
                    }
                    obj.0.borrow_mut().kind = ObjectKind::Array(ArrayData {
                        length: new_len as u32,
                    });
                }
            } else if key
                .as_string()
                .map(|s| s.to_string_lossy() == "length")
                .unwrap_or(false)
            {
                let new_len_val =
                    desc.value
                        .clone()
                        .unwrap_or(Value::Number(match &obj.borrow().kind {
                            ObjectKind::Array(a) => a.length as f64,
                            _ => 0.0,
                        }));
                let new_len_num = self.to_number(&new_len_val)?;
                let new_len = self.to_uint32(&Value::Number(new_len_num))?;
                if new_len as f64 != new_len_num || new_len_num.is_nan() {
                    return self.throw_range_error("Invalid array length");
                }
                let old_len = match &obj.borrow().kind {
                    ObjectKind::Array(a) => a.length,
                    _ => 0,
                };
                if new_len < old_len {
                    let keys: Vec<PropertyKey> = obj
                        .0
                        .borrow()
                        .properties
                        .keys()
                        .filter(|k| k.as_array_index().map(|i| i >= new_len).unwrap_or(false))
                        .cloned()
                        .collect();
                    let mut removed = Vec::new();
                    for k in keys {
                        let configurable = obj
                            .0
                            .borrow()
                            .properties
                            .get(&k)
                            .map(|p| p.configurable)
                            .unwrap_or(true);
                        if !configurable {
                            let len = k.as_array_index().unwrap() + 1;
                            obj.0.borrow_mut().kind = ObjectKind::Array(ArrayData { length: len });
                            return Ok(false);
                        }
                        removed.push(k);
                    }
                    for k in removed {
                        obj.0.borrow_mut().properties.shift_remove(&k);
                    }
                }
                obj.0.borrow_mut().kind = ObjectKind::Array(ArrayData { length: new_len });
                let writable_now = match obj.0.borrow().properties.get(&key) {
                    Some(p) => p.writable,
                    None => true,
                };
                if let Some(w) = desc.writable {
                    if !w {
                        obj.0.borrow_mut().properties.insert(
                            key,
                            Property {
                                value: PropertyValue::Data(Value::Number(new_len as f64)),
                                writable: false,
                                enumerable: false,
                                configurable: false,
                            },
                        );
                    }
                }
                let _ = writable_now;
                return Ok(true);
            }
        }

        // String exotic objects restrict index and length definitions.
        if let ObjectKind::StringObject(s) = &obj.borrow().kind {
            if let Some(idx) = key.as_array_index() {
                if (idx as usize) < s.len() {
                    let current = obj.0.borrow().get_own_property(&key);
                    let same = current
                        .as_ref()
                        .map(|p| {
                            desc.value
                                .as_ref()
                                .map(|v| {
                                    v.as_string()
                                        .map(|vs| vs.as_slice() == &[s.as_slice()[idx as usize]])
                                        .unwrap_or(false)
                                })
                                .unwrap_or(true)
                                && desc.writable.map(|w| w == p.writable).unwrap_or(true)
                                && desc.enumerable.map(|e| e == p.enumerable).unwrap_or(true)
                                && desc
                                    .configurable
                                    .map(|c| c == p.configurable)
                                    .unwrap_or(true)
                        })
                        .unwrap_or(false);
                    if desc.configurable == Some(true)
                        || desc.enumerable == Some(false)
                        || desc.writable == Some(false)
                        || !same
                    {
                        return Ok(false);
                    }
                    return Ok(true);
                }
            }
            if key
                .as_string()
                .map(|k| k.to_string_lossy() == "length")
                .unwrap_or(false)
            {
                return Ok(desc
                    .value
                    .as_ref()
                    .map(|v| v.as_number() == Some(s.len() as f64))
                    .unwrap_or(true)
                    && desc.writable.map(|w| !w).unwrap_or(true)
                    && desc.enumerable.map(|e| !e).unwrap_or(true)
                    && desc.configurable.map(|c| !c).unwrap_or(true));
            }
        }

        // TypedArray exotic behavior.
        let ta_kind = obj.borrow().kind.clone();
        if let ObjectKind::TypedArray(ta) = ta_kind {
            if let Some(idx) = key.as_array_index() {
                if (idx as usize) >= ta.length {
                    return Ok(false);
                }
                let current = obj.0.borrow().get_own_property(&key).unwrap();
                if desc.get.is_some()
                    || desc.set.is_some()
                    || desc.configurable == Some(false)
                    || desc.enumerable == Some(false)
                {
                    return Ok(false);
                }
                if let Some(w) = desc.writable {
                    if !w {
                        return Ok(false);
                    }
                }
                if let Some(v) = desc.value {
                    let _ = current;
                    let n = if ta.kind.is_bigint() {
                        let b = self.to_bigint(&v)?;
                        let i = b.to_i64().unwrap_or_else(|| {
                            if b.is_negative() {
                                i64::MIN
                            } else {
                                i64::MAX
                            }
                        });
                        i as f64
                    } else {
                        self.to_number(&v)?
                    };
                    ta.set_element(idx as usize, n);
                }
                return Ok(true);
            }
            if let Some(prop) = obj.0.borrow().properties.get(&key).cloned() {
                return self.validate_and_apply(obj, &key, desc, Some(&prop));
            }
            return self.validate_and_apply(obj, &key, desc, None);
        }

        let current = obj.0.borrow().get_own_property(&key);
        self.validate_and_apply(obj, &key, desc, current.as_ref())
    }

    fn validate_and_apply(
        &mut self,
        obj: &JsObject,
        key: &PropertyKey,
        desc: Descriptor,
        current: Option<&Property>,
    ) -> JsResult<bool> {
        if current.is_none() {
            if !obj.0.borrow().extensible {
                return Ok(false);
            }
            let prop = if desc.is_accessor_descriptor() {
                Property {
                    value: PropertyValue::Accessor {
                        get: desc.get.clone().unwrap_or(None),
                        set: desc.set.clone().unwrap_or(None),
                    },
                    writable: false,
                    enumerable: desc.enumerable.unwrap_or(false),
                    configurable: desc.configurable.unwrap_or(false),
                }
            } else {
                Property {
                    value: PropertyValue::Data(desc.value.clone().unwrap_or(Value::Undefined)),
                    writable: desc.writable.unwrap_or(false),
                    enumerable: desc.enumerable.unwrap_or(false),
                    configurable: desc.configurable.unwrap_or(false),
                }
            };
            obj.0.borrow_mut().properties.insert(key.clone(), prop);
            return Ok(true);
        }
        let current = current.unwrap();
        if desc.is_generic_descriptor() && desc.enumerable.is_none() && desc.configurable.is_none()
        {
            return Ok(true);
        }
        if !current.configurable {
            if desc.configurable == Some(true) {
                return Ok(false);
            }
            if let Some(e) = desc.enumerable {
                if e != current.enumerable {
                    return Ok(false);
                }
            }
            if !desc.is_generic_descriptor()
                && desc.is_accessor_descriptor() != current.value.is_accessor()
            {
                return Ok(false);
            }
            if let PropertyValue::Accessor { get, set } = &current.value {
                if let Some(g) = &desc.get {
                    if !same_optional_object(g, get) {
                        return Ok(false);
                    }
                }
                if let Some(s) = &desc.set {
                    if !same_optional_object(s, set) {
                        return Ok(false);
                    }
                }
            } else if !current.writable {
                if desc.writable == Some(true) {
                    return Ok(false);
                }
                if let Some(v) = &desc.value {
                    if !Value::same_value(v, current.value.as_data().unwrap()) {
                        return Ok(false);
                    }
                }
            }
        }
        let mut new_prop = current.clone();
        if desc.is_accessor_descriptor() {
            let (mut get, mut set) = match &current.value {
                PropertyValue::Accessor { get, set } => (get.clone(), set.clone()),
                _ => (None, None),
            };
            if let Some(g) = &desc.get {
                get = g.clone();
            }
            if let Some(s) = &desc.set {
                set = s.clone();
            }
            new_prop.value = PropertyValue::Accessor { get, set };
            new_prop.writable = false;
        } else {
            if let Some(v) = &desc.value {
                new_prop.value = PropertyValue::Data(v.clone());
            }
            if let Some(w) = desc.writable {
                new_prop.writable = w;
            }
        }
        if let Some(e) = desc.enumerable {
            new_prop.enumerable = e;
        }
        if let Some(c) = desc.configurable {
            new_prop.configurable = c;
        }
        obj.0.borrow_mut().properties.insert(key.clone(), new_prop);
        Ok(true)
    }

    pub fn set_property(
        &mut self,
        value: &Value,
        key: PropertyKey,
        new_value: Value,
        throw: bool,
    ) -> JsResult<bool> {
        self.check_deadline()?;
        match value {
            Value::Object(obj) => self.set_property_object(obj, key, new_value, value, throw),
            Value::Undefined | Value::Null => {
                if throw {
                    self.throw_type_error("Cannot set property of null or undefined")?;
                }
                Ok(false)
            }
            _ => {
                if throw {
                    return self.throw_type_error("Cannot create property on primitive value");
                }
                Ok(false)
            }
        }
    }

    pub fn set_property_object(
        &mut self,
        obj: &JsObject,
        key: PropertyKey,
        new_value: Value,
        receiver: &Value,
        throw: bool,
    ) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_set(&p, &key, new_value, receiver);
        }
        let own = obj.0.borrow().get_own_property(&key);
        if let Some(prop) = own {
            return match prop.value {
                PropertyValue::Data(_) => {
                    if !prop.writable {
                        if throw {
                            return self.throw_type_error("Cannot assign to read only property");
                        }
                        return Ok(false);
                    }
                    if let Value::Object(recv) = receiver {
                        if recv.ptr_eq(obj) {
                            self.ordinary_define_own_property(
                                obj,
                                key,
                                Descriptor {
                                    value: Some(new_value),
                                    ..Default::default()
                                },
                            )?;
                            return Ok(true);
                        }
                    }
                    let recv_obj = self.to_object(receiver)?;
                    self.create_data_property_or_throw(&recv_obj, key, new_value)?;
                    Ok(true)
                }
                PropertyValue::Accessor { set, .. } => match set {
                    Some(setter) => {
                        self.call_function(&Value::Object(setter), receiver, &[new_value])?;
                        Ok(true)
                    }
                    None => {
                        if throw {
                            return self
                                .throw_type_error("Cannot set property which has only a getter");
                        }
                        Ok(false)
                    }
                },
            };
        }
        let proto = obj.0.borrow().prototype.clone();
        if let Some(proto) = proto {
            let inherited = self.find_property(&proto, &key);
            if let Some(p) = inherited {
                if p.value.is_accessor() {
                    if let PropertyValue::Accessor { set, .. } = p.value {
                        return match set {
                            Some(setter) => {
                                self.call_function(&Value::Object(setter), receiver, &[new_value])?;
                                Ok(true)
                            }
                            None => {
                                if throw {
                                    return self.throw_type_error(
                                        "Cannot set property which has only a getter",
                                    );
                                }
                                Ok(false)
                            }
                        };
                    }
                } else if !p.writable {
                    if throw {
                        return self.throw_type_error("Cannot assign to read only property");
                    }
                    return Ok(false);
                }
            }
        }
        let recv_obj = match receiver {
            Value::Object(o) => o.clone(),
            _ => return Ok(false),
        };
        let existing = recv_obj.0.borrow().get_own_property(&key);
        if existing.is_some() {
            return self.set_property_object(&recv_obj, key, new_value, receiver, throw);
        }
        if !recv_obj.0.borrow().extensible {
            if throw {
                return self.throw_type_error("Cannot add property, object is not extensible");
            }
            return Ok(false);
        }
        self.ordinary_define_own_property(
            &recv_obj,
            key,
            Descriptor {
                value: Some(new_value),
                writable: Some(true),
                enumerable: Some(true),
                configurable: Some(true),
                ..Default::default()
            },
        )
    }

    fn find_property(&mut self, obj: &JsObject, key: &PropertyKey) -> Option<Property> {
        let mut cur = obj.clone();
        loop {
            if let Some(p) = cur.0.borrow().get_own_property(key) {
                return Some(p);
            }
            let proto = cur.0.borrow().prototype.clone();
            match proto {
                Some(p) => cur = p,
                None => return None,
            }
        }
    }

    pub fn delete_property(&mut self, obj: &JsObject, key: &PropertyKey) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_delete(&p, key);
        }
        if let ObjectKind::Array(_) = obj.borrow().kind {
            if key
                .as_string()
                .map(|s| s.to_string_lossy() == "length")
                .unwrap_or(false)
            {
                return Ok(false);
            }
        }
        let ta_kind = obj.borrow().kind.clone();
        if let ObjectKind::TypedArray(ta) = ta_kind {
            if let Some(idx) = key.as_array_index() {
                if (idx as usize) < ta.length {
                    return Ok(false);
                }
            }
        }
        let removed = obj.0.borrow_mut().properties.shift_remove(key);
        match removed {
            Some(p) => Ok(p.configurable),
            None => Ok(true),
        }
    }

    pub fn own_property_keys(&mut self, obj: &JsObject) -> JsResult<Vec<PropertyKey>> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_own_keys(&p);
        }
        let data = obj.0.borrow();
        let mut indices: Vec<u32> = Vec::new();
        let mut strings: Vec<PropertyKey> = Vec::new();
        let mut symbols: Vec<PropertyKey> = Vec::new();
        match &data.kind {
            ObjectKind::StringObject(s) => {
                for i in 0..s.len() {
                    indices.push(i as u32);
                }
                for k in data.properties.keys() {
                    if k.as_array_index()
                        .map(|i| (i as usize) < s.len())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    match k {
                        PropertyKey::String(_) => strings.push(k.clone()),
                        PropertyKey::Symbol(_) => symbols.push(k.clone()),
                    }
                }
                strings.insert(0, PropertyKey::from_str("length"));
            }
            ObjectKind::TypedArray(ta) => {
                for i in 0..ta.length {
                    indices.push(i as u32);
                }
                for k in data.properties.keys() {
                    match k {
                        PropertyKey::String(_) => strings.push(k.clone()),
                        PropertyKey::Symbol(_) => symbols.push(k.clone()),
                    }
                }
            }
            _ => {
                for k in data.properties.keys() {
                    if let Some(i) = k.as_array_index() {
                        indices.push(i);
                    } else {
                        match k {
                            PropertyKey::String(_) => strings.push(k.clone()),
                            PropertyKey::Symbol(_) => symbols.push(k.clone()),
                        }
                    }
                }
                indices.sort_unstable();
            }
        }
        let mut out = Vec::with_capacity(indices.len() + strings.len() + symbols.len());
        for i in indices {
            out.push(PropertyKey::String(JsString::from_str(&i.to_string())));
        }
        out.extend(strings);
        out.extend(symbols);
        Ok(out)
    }

    pub fn get_prototype_of(&mut self, obj: &JsObject) -> JsResult<Option<JsObject>> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_get_prototype_of(&p);
        }
        Ok(obj.0.borrow().prototype.clone())
    }

    pub fn set_prototype_of(&mut self, obj: &JsObject, proto: Option<JsObject>) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_set_prototype_of(&p, proto);
        }
        if let Some(p) = &proto {
            let mut cur = Some(p.clone());
            while let Some(c) = cur {
                if c.ptr_eq(obj) {
                    return Ok(false);
                }
                cur = c.0.borrow().prototype.clone();
            }
        }
        if !obj.0.borrow().extensible {
            return Ok(false);
        }
        obj.0.borrow_mut().prototype = proto;
        Ok(true)
    }

    pub fn is_extensible(&mut self, obj: &JsObject) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_is_extensible(&p);
        }
        Ok(obj.0.borrow().extensible)
    }

    pub fn prevent_extensions(&mut self, obj: &JsObject) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            return self.proxy_prevent_extensions(&p);
        }
        obj.0.borrow_mut().extensible = false;
        Ok(true)
    }

    pub fn test_integrity_level(&mut self, obj: &JsObject) -> JsResult<bool> {
        if let ObjectKind::Proxy(p) = obj.borrow().kind.clone() {
            let target = p.target.clone();
            return Ok(!self.is_extensible(&target)? && self.test_integrity_level_no_ext(&target)?);
        }
        Ok(!obj.0.borrow().extensible && self.all_props_frozen(obj, true)?)
    }

    fn test_integrity_level_no_ext(&mut self, obj: &JsObject) -> JsResult<bool> {
        self.all_props_frozen(obj, true)
    }

    fn all_props_frozen(&mut self, obj: &JsObject, frozen: bool) -> JsResult<bool> {
        let keys = self.own_property_keys(obj)?;
        for k in keys {
            if let Some(p) = self.get_own_property(obj, &k)? {
                if !p.configurable {
                    if p.value.is_accessor() {
                        if frozen {
                            continue;
                        }
                    } else if frozen && p.writable {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub fn to_property_descriptor(&mut self, value: &Value) -> JsResult<Descriptor> {
        let obj = match value {
            Value::Object(o) => o.clone(),
            _ => return self.throw_type_error("Property description must be an object"),
        };
        let mut desc = Descriptor::default();
        let has = |interp: &mut Self, key: &str| -> JsResult<bool> {
            interp.has_property(&obj, &PropertyKey::from_str(key))
        };
        if has(self, "enumerable")? {
            let v = self.get_property_key(&Value::Object(obj.clone()), "enumerable")?;
            desc.enumerable = Some(self.to_boolean(&v));
        }
        if has(self, "configurable")? {
            let v = self.get_property_key(&Value::Object(obj.clone()), "configurable")?;
            desc.configurable = Some(self.to_boolean(&v));
        }
        if has(self, "value")? {
            let v = self.get_property_key(&Value::Object(obj.clone()), "value")?;
            desc.value = Some(v);
        }
        if has(self, "writable")? {
            let v = self.get_property_key(&Value::Object(obj.clone()), "writable")?;
            desc.writable = Some(self.to_boolean(&v));
        }
        if has(self, "get")? {
            let v = self.get_property_key(&Value::Object(obj.clone()), "get")?;
            let get = if v.is_undefined() {
                None
            } else if let Value::Object(o) = &v {
                if o.is_callable() {
                    Some(o.clone())
                } else {
                    return self.throw_type_error("Getter must be a function");
                }
            } else {
                return self.throw_type_error("Getter must be a function");
            };
            desc.get = Some(get);
        }
        if has(self, "set")? {
            let v = self.get_property_key(&Value::Object(obj.clone()), "set")?;
            let set = if v.is_undefined() {
                None
            } else if let Value::Object(o) = &v {
                if o.is_callable() {
                    Some(o.clone())
                } else {
                    return self.throw_type_error("Setter must be a function");
                }
            } else {
                return self.throw_type_error("Setter must be a function");
            };
            desc.set = Some(set);
        }
        if desc.is_accessor_descriptor() && desc.is_data_descriptor() {
            return self.throw_type_error("Invalid property descriptor");
        }
        Ok(desc)
    }

    pub fn from_property_descriptor(&mut self, prop: &Property) -> JsResult<JsObject> {
        let obj = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        match &prop.value {
            PropertyValue::Data(v) => {
                self.create_data_property(&obj, PropertyKey::from_str("value"), v.clone())?;
                self.create_data_property(
                    &obj,
                    PropertyKey::from_str("writable"),
                    Value::Bool(prop.writable),
                )?;
            }
            PropertyValue::Accessor { get, set } => {
                self.create_data_property(
                    &obj,
                    PropertyKey::from_str("get"),
                    get.clone().map(Value::Object).unwrap_or(Value::Undefined),
                )?;
                self.create_data_property(
                    &obj,
                    PropertyKey::from_str("set"),
                    set.clone().map(Value::Object).unwrap_or(Value::Undefined),
                )?;
            }
        }
        self.create_data_property(
            &obj,
            PropertyKey::from_str("enumerable"),
            Value::Bool(prop.enumerable),
        )?;
        self.create_data_property(
            &obj,
            PropertyKey::from_str("configurable"),
            Value::Bool(prop.configurable),
        )?;
        Ok(obj)
    }

    // ------------------------------------------------------------------
    // Iterators
    // ------------------------------------------------------------------
    pub fn get_iterator(&mut self, value: &Value) -> JsResult<Value> {
        let method = self.get_method(
            value,
            &PropertyKey::Symbol(self.realm.intrinsics.symbol_iterator.clone()),
        )?;
        let method = match method {
            Some(m) => m,
            None => return self.throw_type_error("Value is not iterable"),
        };
        let iterator = self.call_function(&method, value, &[])?;
        if !iterator.is_object() {
            return self.throw_type_error("Iterator is not an object");
        }
        let next = self.get_property(&iterator, &PropertyKey::from_str("next"))?;
        let record = JsObject::new(ObjectData {
            prototype: Some(self.realm.intrinsics.iterator_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Iterator(IteratorData {
                iterable: iterator,
                next_method: next,
                done: false,
            }),
            private: IndexMap::new(),
        });
        Ok(Value::Object(record))
    }

    pub fn get_method(&mut self, value: &Value, key: &PropertyKey) -> JsResult<Option<Value>> {
        let method = self.get_property(value, key)?;
        if method.is_nullish() {
            return Ok(None);
        }
        if method
            .as_object_cloned()
            .map(|o| o.is_callable())
            .unwrap_or(false)
        {
            Ok(Some(method))
        } else {
            self.throw_type_error("Method is not a function")
        }
    }

    pub fn iterator_next(&mut self, record: &Value) -> JsResult<(bool, Value)> {
        let obj = record
            .as_object_cloned()
            .ok_or(Completion::Normal)
            .or_else(|_| self.throw_type_error("Iterator is not an object"))?;
        let data = {
            let d = obj.0.borrow();
            match &d.kind {
                ObjectKind::Iterator(i) => i.clone(),
                _ => return self.throw_type_error("Iterator record is invalid"),
            }
        };
        let result = self.call_function(&data.next_method, &data.iterable, &[])?;
        let result_obj = match result.as_object_cloned() {
            Some(o) => o,
            None => return self.throw_type_error("Iterator result is not an object"),
        };
        let done = self.get_property_key(&Value::Object(result_obj.clone()), "done")?;
        if self.to_boolean(&done) {
            return Ok((true, Value::Undefined));
        }
        let value = self.get_property_key(&Value::Object(result_obj), "value")?;
        Ok((false, value))
    }

    pub fn iterator_close(
        &mut self,
        record: &Value,
        completion: Option<Completion>,
    ) -> JsResult<()> {
        let obj = match record.as_object_cloned() {
            Some(o) => o,
            None => return Ok(()),
        };
        let data = {
            let d = obj.0.borrow();
            match &d.kind {
                ObjectKind::Iterator(i) => i.clone(),
                _ => return Ok(()),
            }
        };
        let ret = self.get_property(&data.iterable, &PropertyKey::from_str("return"))?;
        if ret.is_nullish() {
            return Ok(());
        }
        if !ret
            .as_object_cloned()
            .map(|o| o.is_callable())
            .unwrap_or(false)
        {
            return Ok(());
        }
        let inner = self.call_function(&ret, &data.iterable, &[]);
        match completion {
            None => {
                inner?;
                Ok(())
            }
            Some(c) => {
                let _ = inner;
                Err(c)
            }
        }
    }

    // ------------------------------------------------------------------
    // Misc helpers
    // ------------------------------------------------------------------
    pub fn create_array_from_list(&mut self, list: Vec<Value>) -> JsResult<JsObject> {
        let len = list.len();
        let arr = JsObject::array(Some(self.realm.intrinsics.array_prototype.clone()));
        for (i, v) in list.into_iter().enumerate() {
            arr.put(PropertyKey::String(JsString::from_str(&i.to_string())), v);
        }
        if let ObjectKind::Array(a) = &mut arr.0.borrow_mut().kind {
            a.length = len as u32;
        }
        Ok(arr)
    }

    pub fn length_of_array_like(&mut self, obj: &JsObject) -> JsResult<f64> {
        let v = self.get_property(
            &Value::Object(obj.clone()),
            &PropertyKey::from_str("length"),
        )?;
        self.to_length(&v)
    }

    pub fn is_callable_value(&self, value: &Value) -> bool {
        value
            .as_object_cloned()
            .map(|o| o.is_callable())
            .unwrap_or(false)
    }

    pub fn species_constructor(&mut self, obj: &JsObject) -> JsResult<Value> {
        let c = self.get_property(
            &Value::Object(obj.clone()),
            &PropertyKey::from_str("constructor"),
        )?;
        if c.is_undefined() {
            return Ok(Value::Undefined);
        }
        if !c.is_object() {
            return self.throw_type_error("constructor is not an object");
        }
        let species = self.get_property(
            &c,
            &PropertyKey::Symbol(self.realm.intrinsics.symbol_species.clone()),
        )?;
        if species.is_nullish() {
            return Ok(Value::Undefined);
        }
        Ok(species)
    }
}

fn list_len(arr: &JsObject) -> usize {
    match &arr.0.borrow().kind {
        ObjectKind::Array(a) => a.length as usize,
        _ => 0,
    }
}

fn same_optional_object(a: &Option<JsObject>, b: &Option<JsObject>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.ptr_eq(y),
        _ => false,
    }
}

pub fn f64_to_int32(n: f64) -> i32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let n = n.trunc();
    let m = n.rem_euclid(4294967296.0);
    let m = if m >= 2147483648.0 {
        m - 4294967296.0
    } else {
        m
    };
    m as i32
}

pub fn f64_to_uint32(n: f64) -> u32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let n = n.trunc();
    let m = n.rem_euclid(4294967296.0);
    m as u32
}

pub fn string_to_number(s: &JsString) -> f64 {
    let text = s.to_string_lossy();
    parse_js_number(&text)
}

pub fn parse_js_number(text: &str) -> f64 {
    let trimmed = text.trim_matches(is_js_whitespace);
    if trimmed.is_empty() {
        return 0.0;
    }
    let (sign, rest) = if let Some(r) = trimmed.strip_prefix('+') {
        (1.0, r)
    } else if let Some(r) = trimmed.strip_prefix('-') {
        (-1.0, r)
    } else {
        (1.0, trimmed)
    };
    if rest.eq_ignore_ascii_case("infinity") {
        return sign * f64::INFINITY;
    }
    if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        return sign * parse_radix(hex, 16).unwrap_or(f64::NAN);
    }
    if let Some(bin) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
        return sign * parse_radix(bin, 2).unwrap_or(f64::NAN);
    }
    if let Some(oct) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
        return sign * parse_radix(oct, 8).unwrap_or(f64::NAN);
    }
    if !is_decimal_literal(rest) {
        return f64::NAN;
    }
    let parsed: f64 = rest.parse().unwrap_or(f64::NAN);
    sign * parsed
}

fn parse_radix(digits: &str, radix: u32) -> Option<f64> {
    if digits.is_empty() {
        return None;
    }
    let mut value = 0.0f64;
    for c in digits.chars() {
        let d = c.to_digit(radix)?;
        value = value * radix as f64 + d as f64;
    }
    Some(value)
}

fn is_decimal_literal(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut i = 0;
    let mut digits = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == bytes.len()
}

pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

pub fn string_to_bigint(s: &JsString) -> Option<BigInt> {
    let text = s.to_string_lossy();
    let t = text.trim_matches(is_js_whitespace);
    if t.is_empty() {
        return Some(BigInt::zero());
    }
    let (sign, t) = if let Some(r) = t.strip_prefix('-') {
        (-1, r)
    } else if let Some(r) = t.strip_prefix('+') {
        (1, r)
    } else {
        (1, t)
    };
    let (radix, digits) = if let Some(r) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, r)
    } else if let Some(r) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        (8, r)
    } else if let Some(r) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (2, r)
    } else {
        (10, t)
    };
    if digits.is_empty() {
        return None;
    }
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    let value = BigInt::parse_bytes(clean.as_bytes(), radix)?;
    Some(if sign < 0 { -value } else { value })
}
