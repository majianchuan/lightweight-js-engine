//! Core JavaScript value representation.

use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// A JavaScript string: a sequence of UTF-16 code units.
///
/// Rust's `String` cannot represent lone surrogates, but ECMAScript strings can.
/// Keeping the raw UTF-16 representation is therefore necessary for full
/// conformance (String.prototype.length, charCodeAt, surrogate handling, ...).
#[derive(Clone)]
pub struct JsString(Rc<[u16]>);

impl JsString {
    pub fn as_array_index(&self) -> Option<u32> {
        array_index_from_string(self)
    }

    pub fn empty() -> Self {
        JsString(Rc::from(Vec::<u16>::new()))
    }

    pub fn from_utf16(units: Vec<u16>) -> Self {
        JsString(units.into())
    }

    pub fn from_str(s: &str) -> Self {
        JsString(s.encode_utf16().collect::<Vec<u16>>().into())
    }

    pub fn from_rc(units: Rc<[u16]>) -> Self {
        JsString(units)
    }

    pub fn as_slice(&self) -> &[u16] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn code_unit_at(&self, i: usize) -> Option<u16> {
        self.0.get(i).copied()
    }

    pub fn substring(&self, start: usize, end: usize) -> JsString {
        let start = start.min(self.0.len());
        let end = end.min(self.0.len()).max(start);
        JsString(self.0[start..end].into())
    }

    pub fn concat(&self, other: &JsString) -> JsString {
        if self.is_empty() {
            return other.clone();
        }
        if other.is_empty() {
            return self.clone();
        }
        let mut v = Vec::with_capacity(self.len() + other.len());
        v.extend_from_slice(&self.0);
        v.extend_from_slice(&other.0);
        JsString(v.into())
    }

    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    /// True if every code unit is ASCII.
    pub fn is_ascii(&self) -> bool {
        self.0.iter().all(|c| *c < 128)
    }

    pub fn as_ascii_str(&self) -> Option<&str> {
        None
    }

    pub fn to_rust_string(&self) -> String {
        self.to_string_lossy()
    }

    pub fn starts_with(&self, other: &[u16]) -> bool {
        self.0.starts_with(other)
    }

    pub fn find_subslice(&self, needle: &[u16], from: usize) -> Option<usize> {
        if needle.is_empty() {
            return Some(from.min(self.0.len()));
        }
        if from > self.0.len() {
            return None;
        }
        self.0[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    }

    pub fn last_index_of(&self, needle: &[u16], from: usize) -> Option<usize> {
        let max = from.min(self.0.len());
        if needle.is_empty() {
            return Some(max);
        }
        if needle.len() > self.0.len() {
            return None;
        }
        let mut i = max;
        while i + needle.len() <= self.0.len() {
            if &self.0[i..i + needle.len()] == needle {
                return Some(i);
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        None
    }

    /// Return the UTF-8 representation when the string is well-formed UTF-16.
    pub fn try_to_utf8(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.0)
    }

    pub fn hash_key(&self) -> u64 {
        let mut h = rustc_hash::FxHasher::default();
        self.0.hash(&mut h);
        h.finish()
    }
}

impl Default for JsString {
    fn default() -> Self {
        JsString::empty()
    }
}

impl PartialEq for JsString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for JsString {}

impl Hash for JsString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

impl fmt::Display for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_lossy())
    }
}

impl From<&str> for JsString {
    fn from(s: &str) -> Self {
        JsString::from_str(s)
    }
}

impl From<String> for JsString {
    fn from(s: String) -> Self {
        JsString::from_str(&s)
    }
}

/// A JavaScript symbol.
#[derive(Clone, Debug)]
pub struct SymbolValue {
    pub id: u64,
    pub description: Option<JsString>,
}

impl SymbolValue {
    pub fn new(id: u64, description: Option<JsString>) -> Self {
        SymbolValue { id, description }
    }
}

impl PartialEq for SymbolValue {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for SymbolValue {}

impl Hash for SymbolValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

pub const WELL_KNOWN_SYMBOL_IDS: [(&str, u64); 13] = [
    ("asyncIterator", 1),
    ("hasInstance", 2),
    ("isConcatSpreadable", 3),
    ("iterator", 4),
    ("match", 5),
    ("matchAll", 6),
    ("replace", 7),
    ("search", 8),
    ("species", 9),
    ("split", 10),
    ("toPrimitive", 11),
    ("toStringTag", 12),
    ("unscopables", 13),
];

/// A JavaScript value.
#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    BigInt(Rc<BigInt>),
    String(JsString),
    Symbol(SymbolValue),
    Object(crate::object::JsObject),
}

impl Value {
    pub fn number(n: f64) -> Self {
        Value::Number(n)
    }
    pub fn string(s: impl Into<JsString>) -> Self {
        Value::String(s.into())
    }
    pub fn int(n: i64) -> Self {
        Value::Number(n as f64)
    }
    pub fn boolean(b: bool) -> Self {
        Value::Bool(b)
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }
    pub fn is_symbol(&self) -> bool {
        matches!(self, Value::Symbol(_))
    }
    pub fn is_bigint(&self) -> bool {
        matches!(self, Value::BigInt(_))
    }
    pub fn as_object(&self) -> Option<&crate::object::JsObject> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_object_cloned(&self) -> Option<crate::object::JsObject> {
        match self {
            Value::Object(o) => Some(o.clone()),
            _ => None,
        }
    }
    pub fn as_string(&self) -> Option<&JsString> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_symbol(&self) -> Option<&SymbolValue> {
        match self {
            Value::Symbol(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bigint(&self) -> Option<&Rc<BigInt>> {
        match self {
            Value::BigInt(b) => Some(b),
            _ => None,
        }
    }

    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::BigInt(_) => "bigint",
            Value::String(_) => "string",
            Value::Symbol(_) => "symbol",
            Value::Object(o) => {
                if o.is_callable() {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }

    /// The truthiness of a value (ToBoolean).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::BigInt(b) => !b.is_zero(),
            Value::String(s) => !s.is_empty(),
            Value::Symbol(_) => true,
            Value::Object(_) => true,
        }
    }

    pub fn same_value(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Undefined, Value::Undefined) => true,
            (Value::Null, Value::Null) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Number(x), Value::Number(y)) => {
                if x.is_nan() && y.is_nan() {
                    true
                } else if *x == 0.0 && *y == 0.0 {
                    x.is_sign_negative() == y.is_sign_negative()
                } else {
                    x == y
                }
            }
            (Value::BigInt(x), Value::BigInt(y)) => x == y,
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Symbol(x), Value::Symbol(y)) => x.id == y.id,
            (Value::Object(x), Value::Object(y)) => x.ptr_eq(y),
            _ => false,
        }
    }

    pub fn same_value_zero(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) if x.is_nan() && y.is_nan() => true,
            (Value::Number(x), Value::Number(y)) if *x == 0.0 && *y == 0.0 => true,
            _ => Value::same_value(a, b),
        }
    }

    pub fn strict_equals(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => x == y,
            _ => Value::same_value(a, b),
        }
    }

    pub fn is_primitive(&self) -> bool {
        !self.is_object()
    }

    pub fn to_debug_string(&self) -> String {
        match self {
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => format_number(*n),
            Value::BigInt(b) => format!("{}n", b),
            Value::String(s) => format!("{:?}", s.to_string_lossy()),
            Value::Symbol(s) => match &s.description {
                Some(d) => format!("Symbol({})", d.to_string_lossy()),
                None => "Symbol()".to_string(),
            },
            Value::Object(o) => o.debug_string(),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_debug_string())
    }
}

/// ECMAScript number-to-string conversion (without side effects).
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let mut buffer = ryu_js::Buffer::new();
    buffer.format(n).to_string()
}

/// A property key: either a string or a symbol.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    String(JsString),
    Symbol(SymbolValue),
}

impl PropertyKey {
    pub fn from_str(s: &str) -> Self {
        PropertyKey::String(JsString::from_str(s))
    }

    pub fn from_string(s: JsString) -> Self {
        PropertyKey::String(s)
    }

    pub fn to_value(&self) -> Value {
        match self {
            PropertyKey::String(s) => Value::String(s.clone()),
            PropertyKey::Symbol(s) => Value::Symbol(s.clone()),
        }
    }

    pub fn as_string(&self) -> Option<&JsString> {
        match self {
            PropertyKey::String(s) => Some(s),
            _ => None,
        }
    }

    /// Is this property key a canonical array index (0 .. 2^32-2)?
    pub fn as_array_index(&self) -> Option<u32> {
        match self {
            PropertyKey::String(s) => array_index_from_string(s),
            _ => None,
        }
    }

    /// Is this key a canonical numeric index (0 .. 2^32-2)?
    pub fn as_index(&self) -> Option<u32> {
        self.as_array_index()
    }

    pub fn is_symbol(&self) -> bool {
        matches!(self, PropertyKey::Symbol(_))
    }
}

impl fmt::Debug for PropertyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyKey::String(s) => write!(f, "{:?}", s.to_string_lossy()),
            PropertyKey::Symbol(s) => write!(
                f,
                "Symbol({:?})",
                s.description.as_ref().map(|d| d.to_string_lossy())
            ),
        }
    }
}

impl From<&str> for PropertyKey {
    fn from(s: &str) -> Self {
        PropertyKey::String(JsString::from_str(s))
    }
}

impl From<String> for PropertyKey {
    fn from(s: String) -> Self {
        PropertyKey::String(JsString::from_str(&s))
    }
}

impl From<JsString> for PropertyKey {
    fn from(s: JsString) -> Self {
        PropertyKey::String(s)
    }
}

pub fn array_index_from_string(s: &JsString) -> Option<u32> {
    let units = s.as_slice();
    if units.is_empty() || units.len() > 10 {
        return None;
    }
    if units.len() > 1 && units[0] == b'0' as u16 {
        return None;
    }
    let mut val: u64 = 0;
    for &u in units {
        if !(u >= b'0' as u16 && u <= b'9' as u16) {
            return None;
        }
        val = val * 10 + (u - b'0' as u16) as u64;
    }
    if val >= 0xFFFF_FFFF {
        return None;
    }
    Some(val as u32)
}

/// Convert an integral double to a canonical property key when possible.
pub fn number_to_string(n: f64) -> JsString {
    JsString::from_str(&format_number(n))
}

#[derive(Clone, Debug)]
pub enum Completion {
    Normal,
    Throw(Value),
    Return(Value),
    Break(Option<JsString>),
    Continue(Option<JsString>),
}

pub type JsResult<T> = Result<T, Completion>;

pub fn throw<T>(value: Value) -> JsResult<T> {
    Err(Completion::Throw(value))
}

pub fn is_negative_zero(n: f64) -> bool {
    n == 0.0 && n.is_sign_negative()
}
