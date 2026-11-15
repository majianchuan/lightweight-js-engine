//! Lexical environments.

use crate::object::JsObject;
use crate::value::{JsString, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct EnvRef(pub Rc<Env>);

impl std::fmt::Debug for EnvRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnvRef")
    }
}

pub struct Env {
    pub parent: Option<EnvRef>,
    pub kind: EnvKind,
}

pub enum EnvKind {
    Declarative(RefCell<Vec<Option<Value>>>),
    Object(JsObject),
    /// `with (obj)` environment.
    With(JsObject),
}

impl EnvRef {
    pub fn declarative(parent: Option<EnvRef>, slots: usize) -> Self {
        EnvRef(Rc::new(Env {
            parent,
            kind: EnvKind::Declarative(RefCell::new(vec![None; slots])),
        }))
    }

    pub fn object_env(parent: Option<EnvRef>, obj: JsObject) -> Self {
        EnvRef(Rc::new(Env {
            parent,
            kind: EnvKind::Object(obj),
        }))
    }

    pub fn with(parent: Option<EnvRef>, obj: JsObject) -> Self {
        EnvRef(Rc::new(Env {
            parent,
            kind: EnvKind::With(obj),
        }))
    }

    pub fn parent(&self) -> Option<EnvRef> {
        self.0.parent.clone()
    }

    pub fn get(&self, slot: usize) -> Option<Value> {
        match &self.0.kind {
            EnvKind::Declarative(slots) => slots.borrow().get(slot).cloned().flatten(),
            _ => None,
        }
    }

    pub fn set(&self, slot: usize, value: Value) {
        if let EnvKind::Declarative(slots) = &self.0.kind {
            let mut s = slots.borrow_mut();
            if slot < s.len() {
                s[slot] = Some(value);
            }
        }
    }

    pub fn initialize(&self, slot: usize, value: Value) {
        self.set(slot, value);
    }

    pub fn slot_is_initialized(&self, slot: usize) -> bool {
        match &self.0.kind {
            EnvKind::Declarative(slots) => slots
                .borrow()
                .get(slot)
                .map(|s| s.is_some())
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Walk up `depth` parent links.
    pub fn ancestor(&self, depth: usize) -> Option<EnvRef> {
        let mut env = self.clone();
        for _ in 0..depth {
            env = env.0.parent.clone()?;
        }
        Some(env)
    }

    pub fn depth_of(&self, target: &EnvRef) -> Option<usize> {
        let mut env = Some(self.clone());
        let mut depth = 0;
        while let Some(e) = env {
            if Rc::ptr_eq(&e.0, &target.0) {
                return Some(depth);
            }
            env = e.0.parent.clone();
            depth += 1;
        }
        None
    }

    pub fn object(&self) -> Option<&JsObject> {
        match &self.0.kind {
            EnvKind::Object(obj) => Some(obj),
            EnvKind::With(obj) => Some(obj),
            _ => None,
        }
    }

    /// Find an identifier in the environment chain. Returns the value and the
    /// environment that owns it.
    pub fn resolve(&self, name: &JsString) -> Option<(Value, EnvRef)> {
        let mut env = Some(self.clone());
        while let Some(e) = env {
            match &e.0.kind {
                EnvKind::Declarative(slots) => {
                    let _ = slots;
                }
                EnvKind::Object(obj) | EnvKind::With(obj) => {
                    let key = crate::value::PropertyKey::String(name.clone());
                    if let Some(p) = obj.own_property(&key) {
                        if let Some(v) = p.value.as_data() {
                            return Some((v.clone(), e.clone()));
                        }
                    }
                    if let Some(proto) = obj.prototype() {
                        // Object environment records use the prototype chain.
                        let _ = proto;
                    }
                }
            }
            env = e.0.parent.clone();
        }
        None
    }
}
