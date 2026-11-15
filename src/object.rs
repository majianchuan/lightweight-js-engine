//! JavaScript object model: property storage, descriptors and exotic objects.

use crate::value::{JsResult, JsString, PropertyKey, SymbolValue, Value};
use crate::vm::Interp;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

#[derive(Clone)]
pub struct JsObject(pub Rc<RefCell<ObjectData>>);

impl PartialEq for JsObject {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for JsObject {}

impl fmt::Debug for JsObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.debug_string())
    }
}

impl JsObject {
    pub fn new(data: ObjectData) -> Self {
        JsObject(Rc::new(RefCell::new(data)))
    }

    pub fn ordinary(prototype: Option<JsObject>) -> Self {
        JsObject::new(ObjectData {
            prototype,
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Ordinary,
            private: IndexMap::new(),
        })
    }

    pub fn array(prototype: Option<JsObject>) -> Self {
        JsObject::new(ObjectData {
            prototype,
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Array(ArrayData { length: 0 }),
            private: IndexMap::new(),
        })
    }

    pub fn ptr_eq(&self, other: &JsObject) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, ObjectData> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, ObjectData> {
        self.0.borrow_mut()
    }

    pub fn prototype(&self) -> Option<JsObject> {
        self.0.borrow().prototype.clone()
    }

    pub fn set_prototype_raw(&self, proto: Option<JsObject>) {
        self.0.borrow_mut().prototype = proto;
    }

    pub fn is_callable(&self) -> bool {
        match &self.0.borrow().kind {
            ObjectKind::Function(_) => true,
            ObjectKind::Proxy(p) => p.target.is_callable(),
            _ => false,
        }
    }

    pub fn is_constructor(&self) -> bool {
        match &self.0.borrow().kind {
            ObjectKind::Function(f) => match f {
                FunctionKind::User(u) => u.proto.kind.is_constructor() && !u.proto.is_arrow,
                FunctionKind::Native(n) => n.construct.is_some(),
                FunctionKind::Bound(b) => b.target.is_constructor(),
            },
            ObjectKind::Proxy(p) => p.target.is_constructor(),
            _ => false,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match &self.0.borrow().kind {
            ObjectKind::Ordinary => "Object",
            ObjectKind::Array(_) => "Array",
            ObjectKind::Arguments(_) => "Arguments",
            ObjectKind::StringObject(_) => "String",
            ObjectKind::NumberObject(_) => "Number",
            ObjectKind::BooleanObject(_) => "Boolean",
            ObjectKind::SymbolObject(_) => "Symbol",
            ObjectKind::BigIntObject(_) => "BigInt",
            ObjectKind::Function(_) => "Function",
            ObjectKind::Proxy(_) => "Proxy",
            ObjectKind::Date(_) => "Date",
            ObjectKind::RegExp(_) => "RegExp",
            ObjectKind::Map(_) => "Map",
            ObjectKind::Set(_) => "Set",
            ObjectKind::WeakMap(_) => "WeakMap",
            ObjectKind::WeakSet(_) => "WeakSet",
            ObjectKind::ArrayBuffer(_) => "ArrayBuffer",
            ObjectKind::SharedArrayBuffer(_) => "SharedArrayBuffer",
            ObjectKind::TypedArray(_) => "TypedArray",
            ObjectKind::DataView(_) => "DataView",
            ObjectKind::Promise(_) => "Promise",
            ObjectKind::Generator(_) => "Generator",
            ObjectKind::AsyncGenerator(_) => "AsyncGenerator",
            ObjectKind::Iterator(_) => "Iterator",
            ObjectKind::ModuleNamespace(_) => "Module",
            ObjectKind::WeakRef(_) => "WeakRef",
            ObjectKind::FinalizationRegistry(_) => "FinalizationRegistry",
            ObjectKind::ShadowRealm(_) => "ShadowRealm",
        }
    }

    pub fn debug_string(&self) -> String {
        let name = self.kind_name();
        format!("[object {}]", name)
    }

    pub fn function_name(&self) -> Option<JsString> {
        let data = self.0.borrow();
        if let ObjectKind::Function(f) = &data.kind {
            Some(f.name().clone())
        } else {
            None
        }
    }

    pub fn native_function(
        prototype: Option<JsObject>,
        name: impl Into<JsString>,
        length: u32,
        func: NativeCall,
        construct: Option<NativeConstruct>,
    ) -> Self {
        let name = name.into();
        let native = NativeFunction {
            name: name.clone(),
            length,
            func,
            construct,
        };
        let mut obj = ObjectData {
            prototype,
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Function(FunctionKind::Native(Rc::new(native))),
            private: IndexMap::new(),
        };
        obj.properties.insert(
            PropertyKey::from_str("length"),
            Property {
                value: PropertyValue::Data(Value::Number(length as f64)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        obj.properties.insert(
            PropertyKey::from_str("name"),
            Property {
                value: PropertyValue::Data(Value::String(name)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        JsObject::new(obj)
    }

    pub fn user_function(
        prototype: Option<JsObject>,
        name: JsString,
        length: u32,
        data: UserFunction,
    ) -> Self {
        let mut obj = ObjectData {
            prototype,
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Function(FunctionKind::User(Rc::new(data))),
            private: IndexMap::new(),
        };
        obj.properties.insert(
            PropertyKey::from_str("length"),
            Property {
                value: PropertyValue::Data(Value::Number(length as f64)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        obj.properties.insert(
            PropertyKey::from_str("name"),
            Property {
                value: PropertyValue::Data(Value::String(name)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        JsObject::new(obj)
    }

    pub fn bound_function(
        prototype: Option<JsObject>,
        target: JsObject,
        bound_this: Value,
        bound_args: Vec<Value>,
    ) -> Self {
        let length = {
            let tl = target
                .0
                .borrow()
                .properties
                .get(&PropertyKey::from_str("length"))
                .and_then(|p| p.value.as_data())
                .and_then(|v| v.as_number())
                .unwrap_or(0.0);
            (tl - bound_args.len() as f64).max(0.0)
        };
        let name = {
            let n = target
                .0
                .borrow()
                .properties
                .get(&PropertyKey::from_str("name"))
                .and_then(|p| p.value.as_data())
                .and_then(|v| v.as_string().cloned())
                .unwrap_or_else(JsString::empty);
            JsString::from_str(&format!("bound {}", n.to_string_lossy()))
        };
        let data = BoundFunction {
            target,
            bound_this,
            bound_args,
            name: name.clone(),
        };
        let mut obj = ObjectData {
            prototype,
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Function(FunctionKind::Bound(Rc::new(data))),
            private: IndexMap::new(),
        };
        obj.properties.insert(
            PropertyKey::from_str("length"),
            Property {
                value: PropertyValue::Data(Value::Number(length)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        obj.properties.insert(
            PropertyKey::from_str("name"),
            Property {
                value: PropertyValue::Data(Value::String(name)),
                writable: false,
                enumerable: false,
                configurable: true,
            },
        );
        JsObject::new(obj)
    }

    /// Insert a plain data property directly (bypassing descriptors). For
    /// internal use during realm initialization.
    pub fn put(&self, key: impl Into<PropertyKey>, value: Value) {
        let key = key.into();
        self.0.borrow_mut().properties.insert(
            key,
            Property {
                value: PropertyValue::Data(value),
                writable: true,
                enumerable: true,
                configurable: true,
            },
        );
    }

    pub fn put_hidden(&self, key: impl Into<PropertyKey>, value: Value) {
        let key = key.into();
        self.0.borrow_mut().properties.insert(
            key,
            Property {
                value: PropertyValue::Data(value),
                writable: true,
                enumerable: false,
                configurable: true,
            },
        );
    }

    /// Defines a non-writable, non-enumerable, non-configurable data property.
    pub fn define_fixed(&self, key: impl Into<PropertyKey>, value: Value) {
        let key = key.into();
        self.0.borrow_mut().properties.insert(
            key,
            Property {
                value: PropertyValue::Data(value),
                writable: false,
                enumerable: false,
                configurable: false,
            },
        );
    }

    pub fn get_hidden(&self, key: &str) -> Option<Value> {
        self.0
            .borrow()
            .properties
            .get(&PropertyKey::from_str(key))
            .and_then(|p| p.value.as_data().cloned())
    }

    pub fn own_property(&self, key: &PropertyKey) -> Option<Property> {
        self.0.borrow().get_own_property(key)
    }
}

#[derive(Clone, Debug)]
pub struct ObjectData {
    pub prototype: Option<JsObject>,
    pub extensible: bool,
    pub properties: IndexMap<PropertyKey, Property>,
    pub kind: ObjectKind,
    /// Private class fields, keyed by private name id.
    pub private: IndexMap<u64, Property>,
}

impl ObjectData {
    /// Retrieve an own property without invoking getters. Handles the exotic
    /// properties that are not stored in the property map.
    pub fn get_own_property(&self, key: &PropertyKey) -> Option<Property> {
        match &self.kind {
            ObjectKind::Array(a) => {
                if key
                    .as_string()
                    .map(|s| s.to_string_lossy() == "length")
                    .unwrap_or(false)
                {
                    return Some(Property {
                        value: PropertyValue::Data(Value::Number(a.length as f64)),
                        writable: true,
                        enumerable: false,
                        configurable: false,
                    });
                }
                self.properties.get(key).cloned()
            }
            ObjectKind::StringObject(s) => {
                if let Some(idx) = key.as_array_index() {
                    if (idx as usize) < s.len() {
                        let ch = JsString::from_utf16(vec![s.code_unit_at(idx as usize).unwrap()]);
                        return Some(Property {
                            value: PropertyValue::Data(Value::String(ch)),
                            writable: false,
                            enumerable: true,
                            configurable: false,
                        });
                    }
                }
                if key
                    .as_string()
                    .map(|s| s.to_string_lossy() == "length")
                    .unwrap_or(false)
                {
                    return Some(Property {
                        value: PropertyValue::Data(Value::Number(s.len() as f64)),
                        writable: false,
                        enumerable: false,
                        configurable: false,
                    });
                }
                self.properties.get(key).cloned()
            }
            ObjectKind::Arguments(a) => {
                if let Some(idx) = key.as_array_index() {
                    if let Some(Some((env, slot))) = a.mapped.get(idx as usize) {
                        let v = env.get(*slot).unwrap_or(Value::Undefined);
                        return Some(Property {
                            value: PropertyValue::Data(v),
                            writable: true,
                            enumerable: true,
                            configurable: true,
                        });
                    }
                }
                self.properties.get(key).cloned()
            }
            ObjectKind::TypedArray(ta) => {
                if let Some(idx) = key.as_array_index() {
                    if (idx as usize) < ta.length {
                        return Some(Property {
                            value: PropertyValue::Data(ta.get_element_value(idx as usize)),
                            writable: true,
                            enumerable: true,
                            configurable: true,
                        });
                    }
                }
                self.properties.get(key).cloned()
            }
            _ => self.properties.get(key).cloned(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Property {
    pub value: PropertyValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl Property {
    pub fn data(value: Value) -> Self {
        Property {
            value: PropertyValue::Data(value),
            writable: true,
            enumerable: true,
            configurable: true,
        }
    }

    pub fn data_with(value: Value, writable: bool, enumerable: bool, configurable: bool) -> Self {
        Property {
            value: PropertyValue::Data(value),
            writable,
            enumerable,
            configurable,
        }
    }

    pub fn accessor(
        get: Option<JsObject>,
        set: Option<JsObject>,
        enumerable: bool,
        configurable: bool,
    ) -> Self {
        Property {
            value: PropertyValue::Accessor { get, set },
            writable: false,
            enumerable,
            configurable,
        }
    }
}

#[derive(Clone, Debug)]
pub enum PropertyValue {
    Data(Value),
    Accessor {
        get: Option<JsObject>,
        set: Option<JsObject>,
    },
}

impl PropertyValue {
    pub fn as_data(&self) -> Option<&Value> {
        match self {
            PropertyValue::Data(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_accessor(&self) -> Option<(&Option<JsObject>, &Option<JsObject>)> {
        match self {
            PropertyValue::Accessor { get, set } => Some((get, set)),
            _ => None,
        }
    }

    pub fn is_accessor(&self) -> bool {
        matches!(self, PropertyValue::Accessor { .. })
    }
}

/// A (possibly partial) property descriptor, as used by defineProperty.
#[derive(Clone, Default, Debug)]
pub struct Descriptor {
    pub value: Option<Value>,
    pub get: Option<Option<JsObject>>,
    pub set: Option<Option<JsObject>>,
    pub writable: Option<bool>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl Descriptor {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_accessor_descriptor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }
    pub fn is_data_descriptor(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }
    pub fn is_generic_descriptor(&self) -> bool {
        !self.is_accessor_descriptor() && !self.is_data_descriptor()
    }
    pub fn to_complete(&self, current: Option<&Property>) -> Property {
        if self.is_accessor_descriptor() {
            let (cg, cs) = match current.map(|p| &p.value) {
                Some(PropertyValue::Accessor { get, set }) => (get.clone(), set.clone()),
                _ => (None, None),
            };
            Property {
                value: PropertyValue::Accessor {
                    get: self.get.clone().unwrap_or(cg),
                    set: self.set.clone().unwrap_or(cs),
                },
                writable: false,
                enumerable: self
                    .enumerable
                    .or_else(|| current.map(|p| p.enumerable))
                    .unwrap_or(false),
                configurable: self
                    .configurable
                    .or_else(|| current.map(|p| p.configurable))
                    .unwrap_or(false),
            }
        } else {
            let cv = current
                .and_then(|p| p.value.as_data().cloned())
                .unwrap_or(Value::Undefined);
            Property {
                value: PropertyValue::Data(self.value.clone().unwrap_or(cv)),
                writable: self
                    .writable
                    .or_else(|| current.map(|p| p.writable))
                    .unwrap_or(false),
                enumerable: self
                    .enumerable
                    .or_else(|| current.map(|p| p.enumerable))
                    .unwrap_or(false),
                configurable: self
                    .configurable
                    .or_else(|| current.map(|p| p.configurable))
                    .unwrap_or(false),
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum ObjectKind {
    Ordinary,
    Array(ArrayData),
    Arguments(ArgumentsData),
    StringObject(JsString),
    NumberObject(f64),
    BooleanObject(bool),
    SymbolObject(SymbolValue),
    BigIntObject(Rc<num_bigint::BigInt>),
    Function(FunctionKind),
    Proxy(ProxyData),
    Date(f64),
    RegExp(RegExpData),
    Map(MapData),
    Set(SetData),
    WeakMap(WeakMapData),
    WeakSet(WeakSetData),
    ArrayBuffer(ArrayBufferData),
    SharedArrayBuffer(ArrayBufferData),
    TypedArray(TypedArrayData),
    DataView(DataViewData),
    Promise(PromiseData),
    Generator(crate::vm::GeneratorData),
    AsyncGenerator(crate::vm::AsyncGeneratorData),
    Iterator(IteratorData),
    ModuleNamespace(ModuleNamespaceData),
    WeakRef(WeakRefData),
    FinalizationRegistry(FinalizationRegistryData),
    ShadowRealm(ShadowRealmData),
}

impl ObjectKind {
    pub fn is_ordinary(&self) -> bool {
        matches!(self, ObjectKind::Ordinary)
    }
}

#[derive(Clone, Debug)]
pub struct ArrayData {
    pub length: u32,
}

#[derive(Clone, Debug)]
pub struct ArgumentsData {
    /// Map from argument index to (environment, slot) for mapped arguments.
    pub mapped: Vec<Option<(crate::env::EnvRef, usize)>>,
    pub parameter_map_len: usize,
}

#[derive(Clone)]
pub enum FunctionKind {
    User(Rc<UserFunction>),
    Native(Rc<NativeFunction>),
    Bound(Rc<BoundFunction>),
}

impl fmt::Debug for FunctionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FunctionKind")
    }
}

impl FunctionKind {
    pub fn name(&self) -> &JsString {
        match self {
            FunctionKind::User(u) => &u.name,
            FunctionKind::Native(n) => &n.name,
            FunctionKind::Bound(b) => &b.name,
        }
    }
}

#[derive(Debug)]
pub struct UserFunction {
    pub proto: std::sync::Arc<crate::bytecode::FunctionProto>,
    pub env: crate::env::EnvRef,
    pub name: JsString,
    /// Home object for super property references.
    pub home: Option<JsObject>,
    /// Arrow functions capture `this` lexically.
    pub lexical_this: Option<Value>,
    pub lexical_new_target: Option<JsObject>,
    /// Instance fields for class constructors.
    pub class_fields: RefCell<Vec<ClassField>>,
}

#[derive(Clone, Debug)]
pub struct ClassField {
    pub key: PropertyKey,
    pub initializer: Option<JsObject>,
    pub is_static: bool,
}

pub struct NativeFunction {
    pub name: JsString,
    pub length: u32,
    pub func: NativeCall,
    pub construct: Option<NativeConstruct>,
}

impl fmt::Debug for NativeFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NativeFunction({})", self.name)
    }
}

#[derive(Debug)]
pub struct BoundFunction {
    pub target: JsObject,
    pub bound_this: Value,
    pub bound_args: Vec<Value>,
    pub name: JsString,
}

pub type NativeCall = Rc<dyn Fn(&mut Interp, &Value, &[Value]) -> JsResult<Value>>;
pub type NativeConstruct = Rc<dyn Fn(&mut Interp, &[Value], &JsObject) -> JsResult<Value>>;

#[derive(Clone, Debug)]
pub struct ProxyData {
    pub target: JsObject,
    pub handler: JsObject,
    pub callable: bool,
    pub constructible: bool,
    pub revoked: Rc<std::cell::Cell<bool>>,
}

#[derive(Clone, Debug)]
pub struct RegExpData {
    pub source: JsString,
    pub flags: JsString,
    pub regex: Option<Rc<regress::Regex>>,
    pub last_index: f64,
}

#[derive(Clone, Debug)]
pub struct MapData {
    pub entries: Vec<(Value, Value)>,
}

#[derive(Clone, Debug)]
pub struct SetData {
    pub entries: Vec<Value>,
}

#[derive(Clone, Debug)]
pub struct WeakMapData {
    pub entries: Vec<(JsObject, Value)>,
}

#[derive(Clone, Debug)]
pub struct WeakSetData {
    pub entries: Vec<JsObject>,
}

#[derive(Clone, Debug)]
pub struct ArrayBufferData {
    pub bytes: Rc<RefCell<Vec<u8>>>,
    pub detached: bool,
    pub max_byte_length: Option<usize>,
    pub resizable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedArrayKind {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
    BigInt64,
    BigUint64,
}

impl TypedArrayKind {
    pub fn element_size(self) -> usize {
        match self {
            TypedArrayKind::Int8 | TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => 1,
            TypedArrayKind::Int16 | TypedArrayKind::Uint16 => 2,
            TypedArrayKind::Int32 | TypedArrayKind::Uint32 | TypedArrayKind::Float32 => 4,
            TypedArrayKind::Float64 | TypedArrayKind::BigInt64 | TypedArrayKind::BigUint64 => 8,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            TypedArrayKind::Int8 => "Int8Array",
            TypedArrayKind::Uint8 => "Uint8Array",
            TypedArrayKind::Uint8Clamped => "Uint8ClampedArray",
            TypedArrayKind::Int16 => "Int16Array",
            TypedArrayKind::Uint16 => "Uint16Array",
            TypedArrayKind::Int32 => "Int32Array",
            TypedArrayKind::Uint32 => "Uint32Array",
            TypedArrayKind::Float32 => "Float32Array",
            TypedArrayKind::Float64 => "Float64Array",
            TypedArrayKind::BigInt64 => "BigInt64Array",
            TypedArrayKind::BigUint64 => "BigUint64Array",
        }
    }
    pub fn is_bigint(self) -> bool {
        matches!(self, TypedArrayKind::BigInt64 | TypedArrayKind::BigUint64)
    }
}

#[derive(Clone, Debug)]
pub struct TypedArrayData {
    pub buffer: JsObject,
    pub byte_offset: usize,
    pub length: usize,
    pub kind: TypedArrayKind,
    pub tracked: bool,
}

impl TypedArrayData {
    fn buffer_bytes(&self) -> Rc<RefCell<Vec<u8>>> {
        match &self.buffer.0.borrow().kind {
            ObjectKind::ArrayBuffer(a) | ObjectKind::SharedArrayBuffer(a) => a.bytes.clone(),
            _ => unreachable!("typed array buffer is not an ArrayBuffer"),
        }
    }

    pub fn get_element_value(&self, idx: usize) -> Value {
        if self.is_detached() {
            return if self.kind.is_bigint() {
                Value::BigInt(std::rc::Rc::new(num_bigint::BigInt::from(0)))
            } else {
                Value::Number(0.0)
            };
        }
        let bytes_rc = self.buffer_bytes();
        let bytes = bytes_rc.borrow();
        let base = self.byte_offset + idx * self.kind.element_size();
        if base + self.kind.element_size() > bytes.len() {
            return Value::Undefined;
        }
        match self.kind {
            TypedArrayKind::BigInt64 => {
                let v = i64::from_le_bytes(bytes[base..base + 8].try_into().unwrap());
                Value::BigInt(std::rc::Rc::new(num_bigint::BigInt::from(v)))
            }
            TypedArrayKind::BigUint64 => {
                let v = u64::from_le_bytes(bytes[base..base + 8].try_into().unwrap());
                Value::BigInt(std::rc::Rc::new(num_bigint::BigInt::from(v)))
            }
            _ => Value::Number(read_typed(&bytes, base, self.kind)),
        }
    }

    pub fn set_element_bigint(&self, idx: usize, value: &num_bigint::BigInt) {
        if self.is_detached() {
            return;
        }
        let bytes_rc = self.buffer_bytes();
        let mut bytes = bytes_rc.borrow_mut();
        let base = self.byte_offset + idx * self.kind.element_size();
        if base + self.kind.element_size() > bytes.len() {
            return;
        }
        let modulus = num_bigint::BigInt::from(1u64) << 64;
        let mut wrapped: num_bigint::BigInt = value % &modulus;
        if wrapped.sign() == num_bigint::Sign::Minus {
            wrapped += &modulus;
        }
        let u = num_traits::ToPrimitive::to_u64(&wrapped).unwrap_or(0);
        match self.kind {
            TypedArrayKind::BigInt64 => {
                bytes[base..base + 8].copy_from_slice(&(u as i64).to_le_bytes());
            }
            TypedArrayKind::BigUint64 => {
                bytes[base..base + 8].copy_from_slice(&u.to_le_bytes());
            }
            _ => {}
        }
    }

    pub fn get_element(&self, idx: usize) -> f64 {
        if self.is_detached() {
            return 0.0;
        }
        let bytes_rc = self.buffer_bytes();
        let bytes = bytes_rc.borrow();
        let base = self.byte_offset + idx * self.kind.element_size();
        if base + self.kind.element_size() > bytes.len() {
            return 0.0;
        }
        read_typed(&bytes, base, self.kind)
    }

    pub fn set_element(&self, idx: usize, value: f64) {
        if self.is_detached() {
            return;
        }
        let bytes_rc = self.buffer_bytes();
        let mut bytes = bytes_rc.borrow_mut();
        let base = self.byte_offset + idx * self.kind.element_size();
        if base + self.kind.element_size() > bytes.len() {
            return;
        }
        write_typed(&mut bytes, base, self.kind, value);
    }

    pub fn is_detached(&self) -> bool {
        match &self.buffer.0.borrow().kind {
            ObjectKind::ArrayBuffer(a) => a.detached,
            _ => false,
        }
    }
}

pub fn read_typed(bytes: &[u8], base: usize, kind: TypedArrayKind) -> f64 {
    match kind {
        TypedArrayKind::Int8 => bytes[base] as i8 as f64,
        TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => bytes[base] as f64,
        TypedArrayKind::Int16 => i16::from_le_bytes([bytes[base], bytes[base + 1]]) as f64,
        TypedArrayKind::Uint16 => u16::from_le_bytes([bytes[base], bytes[base + 1]]) as f64,
        TypedArrayKind::Int32 => i32::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
        ]) as f64,
        TypedArrayKind::Uint32 => u32::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
        ]) as f64,
        TypedArrayKind::Float32 => f32::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
        ]) as f64,
        TypedArrayKind::Float64 => f64::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
            bytes[base + 4],
            bytes[base + 5],
            bytes[base + 6],
            bytes[base + 7],
        ]),
        TypedArrayKind::BigInt64 => i64::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
            bytes[base + 4],
            bytes[base + 5],
            bytes[base + 6],
            bytes[base + 7],
        ]) as f64,
        TypedArrayKind::BigUint64 => u64::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
            bytes[base + 4],
            bytes[base + 5],
            bytes[base + 6],
            bytes[base + 7],
        ]) as f64,
    }
}

pub fn write_typed(bytes: &mut [u8], base: usize, kind: TypedArrayKind, value: f64) {
    let n = if value.is_nan() { 0.0 } else { value };
    let i32v = crate::ops::f64_to_int32(n);
    let u32v = crate::ops::f64_to_uint32(n);
    match kind {
        TypedArrayKind::Int8 => bytes[base] = (i32v & 0xff) as u8,
        TypedArrayKind::Uint8 => bytes[base] = (u32v & 0xff) as u8,
        TypedArrayKind::Uint8Clamped => {
            let v = if value.is_nan() {
                0.0
            } else {
                value.round().clamp(0.0, 255.0)
            };
            bytes[base] = v as u8;
        }
        TypedArrayKind::Int16 => {
            bytes[base..base + 2].copy_from_slice(&((i32v & 0xffff) as u16).to_le_bytes())
        }
        TypedArrayKind::Uint16 => {
            bytes[base..base + 2].copy_from_slice(&((u32v & 0xffff) as u16).to_le_bytes())
        }
        TypedArrayKind::Int32 => bytes[base..base + 4].copy_from_slice(&i32v.to_le_bytes()),
        TypedArrayKind::Uint32 => bytes[base..base + 4].copy_from_slice(&u32v.to_le_bytes()),
        TypedArrayKind::Float32 => {
            bytes[base..base + 4].copy_from_slice(&(value as f32).to_le_bytes())
        }
        TypedArrayKind::Float64 => bytes[base..base + 8].copy_from_slice(&value.to_le_bytes()),
        TypedArrayKind::BigInt64 => {
            bytes[base..base + 8].copy_from_slice(&(value as i64).to_le_bytes())
        }
        TypedArrayKind::BigUint64 => {
            bytes[base..base + 8].copy_from_slice(&(value as u64).to_le_bytes())
        }
    }
}

#[derive(Clone, Debug)]
pub struct DataViewData {
    pub buffer: JsObject,
    pub byte_offset: usize,
    pub byte_length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromiseState {
    Pending,
    Fulfilled,
    Rejected,
}

#[derive(Clone, Debug)]
pub struct PromiseReaction {
    pub capability: Option<PromiseCapability>,
    pub handler: Option<JsObject>,
    pub kind: ReactionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionKind {
    Fulfill,
    Reject,
}

#[derive(Clone, Debug)]
pub struct PromiseCapability {
    pub promise: JsObject,
    pub resolve: JsObject,
    pub reject: JsObject,
}

#[derive(Clone, Debug)]
pub struct PromiseData {
    pub state: PromiseState,
    pub result: Value,
    pub reactions: Vec<PromiseReaction>,
    pub handled: bool,
    pub is_handled: bool,
}

#[derive(Clone, Debug)]
pub struct IteratorData {
    pub iterable: Value,
    pub next_method: Value,
    pub done: bool,
}

#[derive(Clone, Debug)]
pub struct ModuleNamespaceData {
    pub exports: IndexMap<JsString, Value>,
}

#[derive(Clone, Debug)]
pub struct WeakRefData {
    pub target: JsObject,
}

#[derive(Clone, Debug)]
pub struct FinalizationRegistryData {
    pub cleanup: JsObject,
}

#[derive(Clone, Debug)]
pub struct ShadowRealmData;
