//! Bytecode virtual machine.

use crate::bytecode::*;
use crate::env::EnvRef;
use crate::object::*;
use crate::ops::{f64_to_uint32, string_to_bigint};
use crate::realm::Realm;
use crate::value::*;
use indexmap::IndexMap;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub struct Interp {
    pub realm: Realm,
    pub call_depth: usize,
    pub max_call_depth: usize,
    pub active_env: Option<EnvRef>,
    pub active_this: Value,
    pub active_new_target: Option<JsObject>,
    pub active_home: Option<JsObject>,
    pub active_proto: Option<Arc<FunctionProto>>,
    pub stack_overflow: bool,
    pub done_value: Option<Value>,
    pub uncaught: Option<Value>,
    pub last_this: Value,
    pub last_frame: Option<Frame>,
    pub fuel: usize,
    pub deadline: Option<std::time::Instant>,
    pub check_counter: u64,
}

impl Interp {
    pub fn new() -> Self {
        Interp {
            realm: Realm::new(),
            call_depth: 0,
            max_call_depth: 2500,
            active_env: None,
            active_this: Value::Undefined,
            active_new_target: None,
            active_home: None,
            active_proto: None,
            stack_overflow: false,
            done_value: None,
            uncaught: None,
            last_this: Value::Undefined,
            last_frame: None,
            fuel: usize::MAX,
            deadline: None,
            check_counter: 0,
        }
    }

    pub fn global_this(&self) -> Value {
        Value::Object(self.realm.global.clone())
    }

    pub fn run_script(&mut self, proto: Arc<FunctionProto>) -> JsResult<Value> {
        let env = EnvRef::declarative(None, proto.env_slots as usize);
        let frame = Frame {
            proto,
            func: None,
            func_obj: None,
            env: Some(env),
            env_depth: 0,
            stack: Vec::with_capacity(32),
            temps: Vec::new(),
            pc: 0,
            this: self.global_this(),
            new_target: None,
            home: None,
            args: Vec::new(),
            completion: None,
            normal_target: None,
            finally_handler: None,
            strict: true,
            finished: false,
        };
        match self.execute_frame(frame, Resume::Normal) {
            Ok(ExecOutcome::Return(v)) => Ok(v),
            Ok(ExecOutcome::Throw(v)) => Err(Completion::Throw(v)),
            Ok(_) => self.throw_type_error("Invalid script completion"),
            Err(c) => Err(c),
        }
    }

    pub fn call_function(
        &mut self,
        callee: &Value,
        this: &Value,
        args: &[Value],
    ) -> JsResult<Value> {
        let obj = match callee {
            Value::Object(o) if o.is_callable() => o.clone(),
            _ => return self.throw_type_error("Value is not a function"),
        };
        let kind = {
            let data = obj.0.borrow();
            match &data.kind {
                ObjectKind::Function(f) => f.clone(),
                ObjectKind::Proxy(p) => {
                    let p = p.clone();
                    drop(data);
                    return self.proxy_call(&p, this, args);
                }
                _ => return self.throw_type_error("Value is not a function"),
            }
        };
        if self.call_depth >= self.max_call_depth {
            return self.throw_range_error("Maximum call stack size exceeded");
        }
        self.call_depth += 1;
        let result = self.call_function_kind(obj, kind, this.clone(), args);
        self.call_depth -= 1;
        result
    }

    fn call_function_kind(
        &mut self,
        obj: JsObject,
        kind: FunctionKind,
        this: Value,
        args: &[Value],
    ) -> JsResult<Value> {
        match kind {
            FunctionKind::Native(native) => {
                let saved_env = self.active_env.clone();
                let saved_this = self.active_this.clone();
                let saved_nt = self.active_new_target.clone();
                self.active_this = this.clone();
                let result = (native.func)(self, &this, args);
                self.active_env = saved_env;
                self.active_this = saved_this;
                self.active_new_target = saved_nt;
                result
            }
            FunctionKind::Bound(bound) => {
                let mut all_args = bound.bound_args.clone();
                all_args.extend_from_slice(args);
                self.call_function(
                    &Value::Object(bound.target.clone()),
                    &bound.bound_this,
                    &all_args,
                )
            }
            FunctionKind::User(user) => {
                if user.proto.kind.is_constructor() && self.active_new_target.is_none() {
                    // Class constructors cannot be called without new.
                    if matches!(
                        user.proto.kind,
                        ProtoKind::ClassConstructor | ProtoKind::DerivedConstructor
                    ) {
                        return self
                            .throw_type_error("Class constructor cannot be invoked without 'new'");
                    }
                }
                if user.proto.kind.is_generator() && user.proto.kind.is_async() {
                    return self.create_async_generator(user, this, args, &obj);
                }
                if user.proto.kind.is_generator() {
                    return self.create_generator(user, this, args, &obj);
                }
                if user.proto.kind.is_async() {
                    return self.create_async_function(user, this, args, &obj);
                }
                self.execute_user_function(&user, this, args, None, obj)
            }
        }
    }

    fn execute_user_function(
        &mut self,
        user: &Rc<UserFunction>,
        this: Value,
        args: &[Value],
        new_target: Option<JsObject>,
        func_obj: JsObject,
    ) -> JsResult<Value> {
        let proto = user.proto.clone();
        let env = EnvRef::declarative(Some(user.env.clone()), proto.env_slots as usize);
        for i in 0..proto.env_slots as usize {
            // Parameters and vars are initialized by the prologue.
            let _ = i;
        }
        let mut this = this;
        let is_arrow = proto.is_arrow;
        if is_arrow {
            this = user.lexical_this.clone().unwrap_or(Value::Undefined);
        } else if !proto.is_strict {
            if this.is_nullish() {
                this = self.global_this();
            }
        }
        let frame = Frame {
            proto,
            func: Some(user.clone()),
            func_obj: Some(func_obj.clone()),
            env: Some(env),
            env_depth: 0,
            stack: Vec::with_capacity(32),
            temps: Vec::new(),
            pc: 0,
            this,
            new_target: if is_arrow {
                user.lexical_new_target.clone()
            } else {
                new_target
            },
            home: user.home.clone(),
            args: args.to_vec(),
            completion: None,
            normal_target: None,
            finally_handler: None,
            strict: user.proto.is_strict,
            finished: false,
        };
        match self.execute_frame(frame, Resume::Normal) {
            Ok(ExecOutcome::Return(v)) => Ok(v),
            Ok(ExecOutcome::Throw(v)) => Err(Completion::Throw(v)),
            Ok(_) => self.throw_type_error("Invalid function completion"),
            Err(c) => Err(c),
        }
    }

    pub fn construct_function(
        &mut self,
        callee: &Value,
        args: &[Value],
        new_target: Option<&JsObject>,
    ) -> JsResult<Value> {
        let obj = match callee {
            Value::Object(o) if o.is_constructor() => o.clone(),
            _ => return self.throw_type_error("Value is not a constructor"),
        };
        let new_target = new_target.cloned().unwrap_or_else(|| obj.clone());
        if !new_target.is_constructor() {
            return self.throw_type_error("new.target is not a constructor");
        }
        let kind = {
            let data = obj.0.borrow();
            match &data.kind {
                ObjectKind::Function(f) => f.clone(),
                ObjectKind::Proxy(p) => {
                    let p = p.clone();
                    drop(data);
                    return self.proxy_construct(&p, args, &new_target);
                }
                _ => return self.throw_type_error("Value is not a constructor"),
            }
        };
        match kind {
            FunctionKind::Native(native) => {
                let ctor = match &native.construct {
                    Some(c) => c.clone(),
                    None => return self.throw_type_error("Function is not a constructor"),
                };
                ctor(self, args, &new_target)
            }
            FunctionKind::Bound(bound) => {
                let mut all_args = bound.bound_args.clone();
                all_args.extend_from_slice(args);
                let nt = if new_target.ptr_eq(&obj) {
                    bound.target.clone()
                } else {
                    new_target
                };
                self.construct_function(&Value::Object(bound.target.clone()), &all_args, Some(&nt))
            }
            FunctionKind::User(user) => {
                if user.proto.is_arrow
                    || user.proto.kind.is_method() && !user.proto.kind.is_constructor()
                {
                    return self.throw_type_error("Function is not a constructor");
                }
                if matches!(user.proto.kind, ProtoKind::DerivedConstructor) {
                    // Derived constructor allocates `this` in the super call.
                    let frame_this = Value::Undefined;
                    return self.execute_constructor(
                        &user,
                        frame_this,
                        args,
                        Some(new_target.clone()),
                        true,
                        &obj,
                    );
                }
                let proto_obj = self.get_property(
                    &Value::Object(new_target.clone()),
                    &PropertyKey::from_str("prototype"),
                )?;
                let proto = match proto_obj.as_object_cloned() {
                    Some(p) => Some(p),
                    None => Some(self.realm.intrinsics.object_prototype.clone()),
                };
                let this_obj = JsObject::ordinary(proto);
                let result = self.execute_constructor(
                    &user,
                    Value::Object(this_obj.clone()),
                    args,
                    Some(new_target),
                    false,
                    &obj,
                )?;
                if result.is_object() {
                    Ok(result)
                } else {
                    Ok(Value::Object(this_obj))
                }
            }
        }
    }

    fn execute_constructor(
        &mut self,
        user: &Rc<UserFunction>,
        this: Value,
        args: &[Value],
        new_target: Option<JsObject>,
        derived: bool,
        func_obj: &JsObject,
    ) -> JsResult<Value> {
        let proto = user.proto.clone();
        let env = EnvRef::declarative(Some(user.env.clone()), proto.env_slots as usize);
        if !derived {
            self.initialize_class_fields(user, &this)?;
        }
        let frame = Frame {
            proto,
            func: Some(user.clone()),
            func_obj: Some(func_obj.clone()),
            env: Some(env),
            env_depth: 0,
            stack: Vec::with_capacity(32),
            temps: Vec::new(),
            pc: 0,
            this: this.clone(),
            new_target,
            home: user.home.clone(),
            args: args.to_vec(),
            completion: None,
            normal_target: None,
            finally_handler: None,
            strict: true,
            finished: false,
        };
        let outcome = self.execute_frame(frame, Resume::Normal);
        if derived {
            // The derived constructor returns `this` implicitly; the frame's
            // `this` may have been replaced by a super() call.
            match outcome {
                Ok(ExecOutcome::Return(Value::Undefined)) => return Ok(self.last_this.clone()),
                other => match other {
                    Ok(ExecOutcome::Return(v)) if v.is_object() => return Ok(v),
                    Ok(ExecOutcome::Return(_)) => return Ok(self.last_this.clone()),
                    Ok(ExecOutcome::Throw(v)) => return Err(Completion::Throw(v)),
                    Ok(_) => return Ok(self.last_this.clone()),
                    Err(c) => return Err(c),
                },
            }
        }
        match outcome {
            Ok(ExecOutcome::Return(v)) => Ok(v),
            Ok(ExecOutcome::Throw(v)) => Err(Completion::Throw(v)),
            Ok(_) => Ok(Value::Undefined),
            Err(c) => Err(c),
        }
    }

    pub fn execute_frame(
        &mut self,
        mut frame: Frame,
        resume: Resume,
    ) -> Result<ExecOutcome, Completion> {
        match resume {
            Resume::Normal => {}
            Resume::Value(v) => frame.stack.push(v),
            Resume::Throw(v) => {
                let dispatch = self.dispatch_completion(&mut frame, Completion::Throw(v))?;
                if let Some(out) = dispatch {
                    self.last_this = frame.this.clone();
                    self.last_frame = Some(frame.clone());
                    return Ok(out);
                }
            }
            Resume::Return(v) => {
                let dispatch = self.dispatch_completion(&mut frame, Completion::Return(v))?;
                if let Some(out) = dispatch {
                    self.last_this = frame.this.clone();
                    self.last_frame = Some(frame.clone());
                    return Ok(out);
                }
            }
        }

        loop {
            if self.fuel == 0 {
                return self.throw_range_error("Execution limit exceeded");
            }
            self.fuel -= 1;
            if frame.pc >= frame.proto.body.len() {
                self.last_this = frame.this.clone();
                self.last_frame = Some(frame.clone());
                return Ok(ExecOutcome::Return(Value::Undefined));
            }
            let op = frame.proto.body[frame.pc].clone();
            frame.pc += 1;
            match self.step(&mut frame, op) {
                Ok(Some(outcome)) => {
                    self.last_this = frame.this.clone();
                    self.last_frame = Some(frame.clone());
                    return Ok(outcome);
                }
                Ok(None) => {}
                Err(completion) => {
                    let dispatch = self.dispatch_completion(&mut frame, completion)?;
                    if let Some(outcome) = dispatch {
                        self.last_this = frame.this.clone();
                        self.last_frame = Some(frame.clone());
                        return Ok(outcome);
                    }
                }
            }
        }
    }

    fn step(&mut self, frame: &mut Frame, op: Op) -> Result<Option<ExecOutcome>, Completion> {
        match op {
            Op::Nop => {}
            Op::PushUndefined => frame.stack.push(Value::Undefined),
            Op::PushNull => frame.stack.push(Value::Null),
            Op::PushTrue => frame.stack.push(Value::Bool(true)),
            Op::PushFalse => frame.stack.push(Value::Bool(false)),
            Op::PushConst(i) => {
                let value = self.constant_to_value(&frame.proto.constants[i as usize])?;
                frame.stack.push(value);
            }
            Op::Pop => {
                frame.stack.pop();
            }
            Op::Dup => {
                let v = frame.stack.last().cloned().unwrap_or(Value::Undefined);
                frame.stack.push(v);
            }
            Op::Swap => {
                let len = frame.stack.len();
                if len >= 2 {
                    frame.stack.swap(len - 1, len - 2);
                }
            }
            Op::Rot3 => {
                let len = frame.stack.len();
                if len >= 3 {
                    let v = frame.stack.remove(len - 3);
                    frame.stack.push(v);
                }
            }
            Op::LoadVar { depth, slot } => {
                let value = self.load_var(&frame, depth, slot)?;
                frame.stack.push(value);
            }
            Op::StoreVar { depth, slot } => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                self.store_var(&frame, depth, slot, value.clone(), false)?;
                frame.stack.push(value);
            }
            Op::InitVar { depth, slot } => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                self.store_var(&frame, depth, slot, value, true)?;
            }
            Op::LoadGlobal(name_idx) => {
                let name = self.const_string(&frame.proto.constants[name_idx as usize]);
                let value = self.load_global(&name)?;
                frame.stack.push(value);
            }
            Op::StoreGlobal { name, strict } => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let name = self.const_string(&frame.proto.constants[name as usize]);
                self.store_global(&name, value.clone(), strict)?;
                frame.stack.push(value);
            }
            Op::TypeOfGlobal(name_idx) => {
                let name = self.const_string(&frame.proto.constants[name_idx as usize]);
                match self.try_load_global(&name) {
                    Ok(Some(v)) => frame
                        .stack
                        .push(Value::String(JsString::from_str(v.type_of()))),
                    Ok(None) => frame
                        .stack
                        .push(Value::String(JsString::from_str("undefined"))),
                    Err(c) => return Err(c),
                }
            }
            Op::DeclareGlobalVar(name_idx) => {
                let name = self.const_string(&frame.proto.constants[name_idx as usize]);
                self.declare_global_var(&name)?;
            }
            Op::DeleteGlobal(name_idx) => {
                let name = self.const_string(&frame.proto.constants[name_idx as usize]);
                let key = PropertyKey::String(name);
                let result = self.delete_property(&self.realm.global.clone(), &key)?;
                frame.stack.push(Value::Bool(result));
            }
            Op::StoreGlobalFunction(name_idx) => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let name = self.const_string(&frame.proto.constants[name_idx as usize]);
                self.declare_global_function(&name, value)?;
            }
            Op::LoadThis => frame.stack.push(frame.this.clone()),
            Op::LoadNewTarget => frame.stack.push(
                frame
                    .new_target
                    .clone()
                    .map(Value::Object)
                    .unwrap_or(Value::Undefined),
            ),
            Op::LoadArg(i) => frame.stack.push(
                frame
                    .args
                    .get(i as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined),
            ),
            Op::LoadArgsCount => frame.stack.push(Value::Number(frame.args.len() as f64)),
            Op::LoadArgsObject => frame.stack.push(Value::Number(frame.args.len() as f64)),
            Op::CreateRestArray(start) => {
                let start = (start as usize).min(frame.args.len());
                let vals = frame.args[start..].to_vec();
                let arr = self.create_array_from_list(vals)?;
                frame.stack.push(Value::Object(arr));
            }
            Op::CreateArguments => {
                let args = self.create_arguments_object(&frame)?;
                frame.stack.push(args);
            }
            Op::CreateEnv(slots) => {
                let parent = frame.env.clone();
                let env = EnvRef::declarative(parent, slots as usize);
                frame.env = Some(env);
                frame.env_depth += 1;
            }
            Op::CreateWithEnv => {
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = self.to_object(&obj)?;
                let parent = frame.env.clone();
                frame.env = Some(EnvRef::with(parent, obj));
                frame.env_depth += 1;
            }
            Op::LoadName {
                name,
                depth,
                slot,
                has_local,
            } => {
                let name = self.const_string(&frame.proto.constants[name as usize]);
                let key = PropertyKey::String(name.clone());
                let mut found = None;
                let mut env = frame.env.clone();
                while let Some(e) = env {
                    if let crate::env::EnvKind::With(obj) = &e.0.kind {
                        if self.has_property(obj, &key)? {
                            found = Some(obj.clone());
                            break;
                        }
                    }
                    env = e.parent();
                }
                if let Some(obj) = found {
                    frame
                        .stack
                        .push(self.get_property(&Value::Object(obj), &key)?);
                } else if has_local {
                    frame.stack.push(self.load_var(&frame, depth, slot)?);
                } else {
                    let v = self.load_global(&name)?;
                    frame.stack.push(v);
                }
            }
            Op::StoreName {
                name,
                depth,
                slot,
                has_local,
                strict,
            } => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let name = self.const_string(&frame.proto.constants[name as usize]);
                let key = PropertyKey::String(name.clone());
                let mut found = None;
                let mut env = frame.env.clone();
                while let Some(e) = env {
                    if let crate::env::EnvKind::With(obj) = &e.0.kind {
                        if self.has_property(obj, &key)? {
                            found = Some(obj.clone());
                            break;
                        }
                    }
                    env = e.parent();
                }
                if let Some(obj) = found {
                    self.set_property_object(
                        &obj,
                        key,
                        value.clone(),
                        &Value::Object(obj.clone()),
                        strict,
                    )?;
                } else if has_local {
                    self.store_var(&frame, depth, slot, value.clone(), false)?;
                } else {
                    self.store_global(&name, value.clone(), strict)?;
                }
                frame.stack.push(value);
            }
            Op::TypeOfName {
                name,
                depth,
                slot,
                has_local,
            } => {
                let name = self.const_string(&frame.proto.constants[name as usize]);
                let key = PropertyKey::String(name.clone());
                let mut found = None;
                let mut env = frame.env.clone();
                while let Some(e) = env {
                    if let crate::env::EnvKind::With(obj) = &e.0.kind {
                        if self.has_property(obj, &key)? {
                            found = Some(obj.clone());
                            break;
                        }
                    }
                    env = e.parent();
                }
                if let Some(obj) = found {
                    let v = self.get_property(&Value::Object(obj), &key)?;
                    frame
                        .stack
                        .push(Value::String(JsString::from_str(v.type_of())));
                } else if has_local {
                    let v = self.load_var(&frame, depth, slot)?;
                    frame
                        .stack
                        .push(Value::String(JsString::from_str(v.type_of())));
                } else if self.has_property(&self.realm.global.clone(), &key)? {
                    let v = self.get_property(&self.global_this(), &key)?;
                    frame
                        .stack
                        .push(Value::String(JsString::from_str(v.type_of())));
                } else {
                    frame
                        .stack
                        .push(Value::String(JsString::from_str("undefined")));
                }
            }
            Op::PopEnv => {
                if let Some(env) = frame.env.clone() {
                    frame.env = env.parent();
                    frame.env_depth = frame.env_depth.saturating_sub(1);
                }
            }
            Op::CreateClosure(idx) => {
                let closure_proto = frame.proto.functions[idx as usize].clone();
                let func = self.make_closure(closure_proto, &frame);
                frame.stack.push(Value::Object(func));
            }
            Op::CreateClosureWithHome(idx) => {
                let home = frame.stack.pop().unwrap_or(Value::Undefined);
                let closure_proto = frame.proto.functions[idx as usize].clone();
                let home = home.as_object_cloned();
                let func = self.make_closure_with_home(closure_proto, &frame, home);
                frame.stack.push(Value::Object(func));
            }
            Op::NewObject => {
                frame.stack.push(Value::Object(JsObject::ordinary(Some(
                    self.realm.intrinsics.object_prototype.clone(),
                ))));
            }
            Op::NewArray => {
                frame.stack.push(Value::Object(JsObject::array(Some(
                    self.realm.intrinsics.array_prototype.clone(),
                ))));
            }
            Op::ArrayPush => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let arr = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(a) = &arr {
                    let len = match &a.0.borrow().kind {
                        ObjectKind::Array(d) => d.length,
                        _ => 0,
                    };
                    a.put(
                        PropertyKey::String(JsString::from_str(&len.to_string())),
                        value,
                    );
                    if let ObjectKind::Array(d) = &mut a.0.borrow_mut().kind {
                        d.length = len + 1;
                    }
                }
                frame.stack.push(arr);
            }
            Op::ArrayPushSpread => {
                let iterable = frame.stack.pop().unwrap_or(Value::Undefined);
                let arr = frame.stack.pop().unwrap_or(Value::Undefined);
                let iter = self.get_iterator(&iterable)?;
                loop {
                    let (done, value) = self.iterator_next(&iter)?;
                    if done {
                        break;
                    }
                    if let Value::Object(a) = &arr {
                        let len = match &a.0.borrow().kind {
                            ObjectKind::Array(d) => d.length,
                            _ => 0,
                        };
                        a.put(
                            PropertyKey::String(JsString::from_str(&len.to_string())),
                            value,
                        );
                        if let ObjectKind::Array(d) = &mut a.0.borrow_mut().kind {
                            d.length = len + 1;
                        }
                    }
                }
                frame.stack.push(arr);
            }
            Op::SetArrayLength(n) => {
                let arr = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(a) = &arr {
                    if let ObjectKind::Array(d) = &mut a.0.borrow_mut().kind {
                        d.length = d.length.max(n);
                    }
                }
                frame.stack.push(arr);
            }
            Op::DefineDataProp { key, computed } => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = if computed {
                    let k = frame.stack.pop().unwrap_or(Value::Undefined);
                    self.to_property_key(&k)?
                } else {
                    self.const_key(&frame.proto.constants[key as usize])
                };
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(o) = &obj {
                    self.create_data_property_or_throw(o, key, value)?;
                }
                frame.stack.push(obj);
            }
            Op::DefineAccessorProp {
                key,
                computed,
                getter,
            } => {
                let func = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = if computed {
                    let k = frame.stack.pop().unwrap_or(Value::Undefined);
                    self.to_property_key(&k)?
                } else {
                    self.const_key(&frame.proto.constants[key as usize])
                };
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(o) = &obj {
                    let func_obj = func.as_object_cloned();
                    let desc = if getter {
                        Descriptor {
                            get: Some(func_obj),
                            enumerable: Some(true),
                            configurable: Some(true),
                            ..Default::default()
                        }
                    } else {
                        Descriptor {
                            set: Some(func_obj),
                            enumerable: Some(true),
                            configurable: Some(true),
                            ..Default::default()
                        }
                    };
                    self.define_own_property(o, key, desc)?;
                }
                frame.stack.push(obj);
            }
            Op::SetPrototypeOf => {
                let proto = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(o) = &obj {
                    let p = proto.as_object_cloned();
                    let _ = self.set_prototype_of(o, p);
                }
                frame.stack.push(obj);
            }
            Op::DefineClassPrototype => {
                let p = frame.stack.pop().unwrap_or(Value::Undefined);
                let ctor = frame.stack.pop().unwrap_or(Value::Undefined);
                if let (Value::Object(c), Value::Object(p)) = (&ctor, &p) {
                    self.define_own_property(
                        c,
                        PropertyKey::from_str("prototype"),
                        Descriptor {
                            value: Some(Value::Object(p.clone())),
                            writable: Some(false),
                            enumerable: Some(false),
                            configurable: Some(false),
                            ..Default::default()
                        },
                    )?;
                }
                frame.stack.push(ctor);
            }
            Op::DefineClassConstructor => {
                let ctor = frame.stack.pop().unwrap_or(Value::Undefined);
                let p = frame.stack.pop().unwrap_or(Value::Undefined);
                if let (Value::Object(p), Value::Object(c)) = (&p, &ctor) {
                    self.define_own_property(
                        p,
                        PropertyKey::from_str("constructor"),
                        Descriptor {
                            value: Some(Value::Object(c.clone())),
                            writable: Some(true),
                            enumerable: Some(false),
                            configurable: Some(true),
                            ..Default::default()
                        },
                    )?;
                }
                frame.stack.push(p);
            }
            Op::DefineClassMethod { kind } => {
                let closure = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let target = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(t) = &target {
                    let key = self.to_property_key(&key)?;
                    let f = closure.as_object_cloned();
                    let desc = match kind {
                        1 => Descriptor {
                            get: Some(f),
                            enumerable: Some(false),
                            configurable: Some(true),
                            ..Default::default()
                        },
                        2 => Descriptor {
                            set: Some(f),
                            enumerable: Some(false),
                            configurable: Some(true),
                            ..Default::default()
                        },
                        _ => Descriptor {
                            value: Some(closure.clone()),
                            writable: Some(true),
                            enumerable: Some(false),
                            configurable: Some(true),
                            ..Default::default()
                        },
                    };
                    self.define_own_property(t, key, desc)?;
                }
                frame.stack.push(target);
            }
            Op::AddClassField { is_static } => {
                let initializer = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let ctor = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(c) = &ctor {
                    let key = self.to_property_key(&key)?;
                    let init = initializer.as_object_cloned();
                    if is_static {
                        let value = match &init {
                            Some(f) => self.call_function(
                                &Value::Object(f.clone()),
                                &Value::Object(c.clone()),
                                &[],
                            )?,
                            None => Value::Undefined,
                        };
                        self.create_data_property_or_throw(c, key, value)?;
                    } else {
                        let user = {
                            let data = c.borrow();
                            match &data.kind {
                                ObjectKind::Function(FunctionKind::User(u)) => Some(u.clone()),
                                _ => None,
                            }
                        };
                        if let Some(u) = user {
                            u.class_fields.borrow_mut().push(ClassField {
                                key,
                                initializer: init,
                                is_static: false,
                            });
                        }
                    }
                }
                frame.stack.push(ctor);
            }
            Op::PushArgsArray => {
                let arr = self.create_array_from_list(frame.args.clone())?;
                frame.stack.push(Value::Object(arr));
            }
            Op::SetProtoLiteral => {
                let proto = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(o) = &obj {
                    match &proto {
                        Value::Object(p) => {
                            self.set_prototype_of(o, Some(p.clone()))?;
                        }
                        Value::Null => {
                            self.set_prototype_of(o, None)?;
                        }
                        _ => {}
                    }
                }
                frame.stack.push(obj);
            }
            Op::CopyDataProperties => {
                let source = frame.stack.pop().unwrap_or(Value::Undefined);
                let target = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(t) = &target {
                    if !source.is_nullish() {
                        let src = self.to_object(&source)?;
                        let keys = self.own_property_keys(&src)?;
                        for key in keys {
                            let desc = self.get_own_property(&src, &key)?;
                            if let Some(d) = desc {
                                if d.enumerable {
                                    if let PropertyValue::Data(v) = d.value {
                                        self.create_data_property_or_throw(t, key, v)?;
                                    } else if let PropertyValue::Accessor { .. } = d.value {
                                        let v = self.get_property(&source, &key)?;
                                        self.create_data_property_or_throw(t, key, v)?;
                                    }
                                }
                            }
                        }
                    }
                }
                frame.stack.push(target);
            }
            Op::GetProp(key_idx) => {
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.const_key(&frame.proto.constants[key_idx as usize]);
                let v = self.get_property(&obj, &key)?;
                frame.stack.push(v);
            }
            Op::SetProp(key_idx) => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.const_key(&frame.proto.constants[key_idx as usize]);
                self.set_property(&obj, key, value.clone(), frame.strict)?;
                frame.stack.push(value);
            }
            Op::GetElem => {
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.to_property_key(&key)?;
                let v = self.get_property(&obj, &key)?;
                frame.stack.push(v);
            }
            Op::SetElem => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.to_property_key(&key)?;
                self.set_property(&obj, key, value.clone(), frame.strict)?;
                frame.stack.push(value);
            }
            Op::DeleteProp(key_idx) => {
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.const_key(&frame.proto.constants[key_idx as usize]);
                let result = self.delete_property_value(&obj, &key)?;
                frame.stack.push(Value::Bool(result));
            }
            Op::DeleteElem => {
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.to_property_key(&key)?;
                let result = self.delete_property_value(&obj, &key)?;
                frame.stack.push(Value::Bool(result));
            }
            Op::DefineOwnDataProp(key_idx) => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let obj = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.const_key(&frame.proto.constants[key_idx as usize]);
                if let Value::Object(o) = &obj {
                    self.create_data_property_or_throw(o, key, value)?;
                }
                frame.stack.push(obj);
            }
            Op::GetSuperProp(key_idx) => {
                let key = self.const_key(&frame.proto.constants[key_idx as usize]);
                let v = self.get_super_property(&frame, &key)?;
                frame.stack.push(v);
            }
            Op::SetSuperProp(key_idx) => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.const_key(&frame.proto.constants[key_idx as usize]);
                self.set_super_property(&frame, key, value.clone())?;
                frame.stack.push(value);
            }
            Op::GetSuperElem => {
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.to_property_key(&key)?;
                let v = self.get_super_property(&frame, &key)?;
                frame.stack.push(v);
            }
            Op::SetSuperElem => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.to_property_key(&key)?;
                self.set_super_property(&frame, key, value.clone())?;
                frame.stack.push(value);
            }
            Op::PrivateGet(_) | Op::PrivateSet(_) | Op::PrivateIn(_) => {
                return self.throw_syntax_error("Private fields are not supported yet");
            }
            Op::Binary(op) => {
                let right = frame.stack.pop().unwrap_or(Value::Undefined);
                let left = frame.stack.pop().unwrap_or(Value::Undefined);
                let v = self.binary_op(op, &left, &right)?;
                frame.stack.push(v);
            }
            Op::Unary(op) => {
                let v = frame.stack.pop().unwrap_or(Value::Undefined);
                let v = self.unary_op(op, &v)?;
                frame.stack.push(v);
            }
            Op::ToPropertyKey => {
                let v = frame.stack.pop().unwrap_or(Value::Undefined);
                let key = self.to_property_key(&v)?;
                frame.stack.push(key.to_value());
            }
            Op::ToNumeric => {
                let v = frame.stack.pop().unwrap_or(Value::Undefined);
                let n = self.to_numeric(&v)?;
                frame.stack.push(n);
            }
            Op::ToString => {
                let v = frame.stack.pop().unwrap_or(Value::Undefined);
                let s = self.to_string(&v)?;
                frame.stack.push(Value::String(s));
            }
            Op::CopyDataPropertiesExcluding(count) => {
                let mut excluded = Vec::new();
                for _ in 0..count {
                    excluded.push(frame.stack.pop().unwrap_or(Value::Undefined));
                }
                let source = frame.stack.pop().unwrap_or(Value::Undefined);
                let target = frame.stack.pop().unwrap_or(Value::Undefined);
                if let Value::Object(t) = &target {
                    if !source.is_nullish() {
                        let src = self.to_object(&source)?;
                        let keys = self.own_property_keys(&src)?;
                        for key in keys {
                            if excluded
                                .iter()
                                .any(|e| Value::strict_equals(e, &key.to_value()))
                            {
                                continue;
                            }
                            if let Some(d) = self.get_own_property(&src, &key)? {
                                if d.enumerable {
                                    let v = match d.value {
                                        PropertyValue::Data(v) => v,
                                        PropertyValue::Accessor { .. } => {
                                            self.get_property(&source, &key)?
                                        }
                                    };
                                    self.create_data_property_or_throw(t, key, v)?;
                                }
                            }
                        }
                    }
                }
                frame.stack.push(target);
            }
            Op::Jump(target) => {
                frame.pc = target as usize;
            }
            Op::JumpIfFalse(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if !cond.truthy() {
                    frame.pc = target as usize;
                }
            }
            Op::JumpIfTrue(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if cond.truthy() {
                    frame.pc = target as usize;
                }
            }
            Op::JumpIfFalsePop(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if !cond.truthy() {
                    frame.pc = target as usize;
                }
            }
            Op::JumpIfTruePop(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if cond.truthy() {
                    frame.pc = target as usize;
                }
            }
            Op::JumpIfNullishPop(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if cond.is_nullish() {
                    frame.pc = target as usize;
                }
            }
            Op::JumpIfNotNullishPop(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if !cond.is_nullish() {
                    frame.pc = target as usize;
                }
            }
            Op::JumpIfNotUndefined(target) => {
                let cond = frame.stack.pop().unwrap_or(Value::Undefined);
                if !cond.is_undefined() {
                    frame.pc = target as usize;
                }
            }
            Op::Call(argc) => {
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    args.push(frame.stack.pop().unwrap_or(Value::Undefined));
                }
                args.reverse();
                let this = frame.stack.pop().unwrap_or(Value::Undefined);
                let callee = frame.stack.pop().unwrap_or(Value::Undefined);
                let result = self.call_function(&callee, &this, &args)?;
                frame.stack.push(result);
            }
            Op::CallSpread => {
                let args_arr = frame.stack.pop().unwrap_or(Value::Undefined);
                let this = frame.stack.pop().unwrap_or(Value::Undefined);
                let callee = frame.stack.pop().unwrap_or(Value::Undefined);
                let args = self.iterable_to_list(&args_arr)?;
                let result = self.call_function(&callee, &this, &args)?;
                frame.stack.push(result);
            }
            Op::New(argc) => {
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    args.push(frame.stack.pop().unwrap_or(Value::Undefined));
                }
                args.reverse();
                let callee = frame.stack.pop().unwrap_or(Value::Undefined);
                let result = self.construct_function(&callee, &args, None)?;
                frame.stack.push(result);
            }
            Op::NewSpread => {
                let args_arr = frame.stack.pop().unwrap_or(Value::Undefined);
                let callee = frame.stack.pop().unwrap_or(Value::Undefined);
                let args = self.iterable_to_list(&args_arr)?;
                let result = self.construct_function(&callee, &args, None)?;
                frame.stack.push(result);
            }
            Op::SuperCall(argc) => {
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    args.push(frame.stack.pop().unwrap_or(Value::Undefined));
                }
                args.reverse();
                let result = self.super_construct(frame, &args)?;
                frame.stack.push(result);
            }
            Op::SuperCallSpread => {
                let args_arr = frame.stack.pop().unwrap_or(Value::Undefined);
                let args = self.iterable_to_list(&args_arr)?;
                let result = self.super_construct(frame, &args)?;
                frame.stack.push(result);
            }
            Op::Return => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                return Err(Completion::Return(value));
            }
            Op::ReturnUndefined => {
                return Err(Completion::Return(Value::Undefined));
            }
            Op::Throw => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                return Err(Completion::Throw(value));
            }
            Op::LoadTemp(i) => {
                let v = frame
                    .temps
                    .get(i as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                frame.stack.push(v);
            }
            Op::StoreTemp(i) => {
                let v = frame.stack.pop().unwrap_or(Value::Undefined);
                if frame.temps.len() <= i as usize {
                    frame.temps.resize(i as usize + 1, Value::Undefined);
                }
                frame.temps[i as usize] = v;
            }
            Op::StoreTempKeep(i) => {
                let v = frame.stack.last().cloned().unwrap_or(Value::Undefined);
                if frame.temps.len() <= i as usize {
                    frame.temps.resize(i as usize + 1, Value::Undefined);
                }
                frame.temps[i as usize] = v;
            }
            Op::IterInit(temp) => {
                let iterable = frame.stack.pop().unwrap_or(Value::Undefined);
                let iter = self.get_iterator(&iterable)?;
                if frame.temps.len() <= temp as usize {
                    frame.temps.resize(temp as usize + 1, Value::Undefined);
                }
                frame.temps[temp as usize] = iter;
            }
            Op::AsyncIterInit(temp) => {
                let iterable = frame.stack.pop().unwrap_or(Value::Undefined);
                let async_method = self.get_method(
                    &iterable,
                    &PropertyKey::Symbol(self.realm.intrinsics.symbol_async_iterator.clone()),
                )?;
                let record = if let Some(m) = async_method {
                    let iter = self.call_function(&m, &iterable, &[])?;
                    let iter_obj = match iter.as_object_cloned() {
                        Some(o) => o,
                        None => return self.throw_type_error("Iterator is not an object"),
                    };
                    let next = self.get_property(
                        &Value::Object(iter_obj.clone()),
                        &PropertyKey::from_str("next"),
                    )?;
                    let rec =
                        JsObject::ordinary(Some(self.realm.intrinsics.iterator_prototype.clone()));
                    rec.put_hidden("__async_iter", iter);
                    rec.put_hidden("__async_next", next);
                    rec
                } else {
                    let rec = self.get_iterator(&iterable)?;
                    rec.as_object_cloned().unwrap()
                };
                if frame.temps.len() <= temp as usize {
                    frame.temps.resize(temp as usize + 1, Value::Undefined);
                }
                frame.temps[temp as usize] = Value::Object(record);
            }
            Op::AsyncIterNext(temp) => {
                let record = frame
                    .temps
                    .get(temp as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                let obj = match record.as_object_cloned() {
                    Some(o) => o,
                    None => return self.throw_type_error("Iterator is not an object"),
                };
                if let Some(next) = obj.get_hidden("__async_next") {
                    let iter = obj.get_hidden("__async_iter").unwrap_or(Value::Undefined);
                    let r = self.call_function(&next, &iter, &[])?;
                    frame.stack.push(r);
                } else {
                    let (done, value) = self.iterator_next(&record)?;
                    let result =
                        JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
                    result.put_hidden("value", value);
                    result.put_hidden("done", Value::Bool(done));
                    frame.stack.push(Value::Object(result));
                }
            }
            Op::IterNext { temp, done } => {
                let iter = frame
                    .temps
                    .get(temp as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                let (is_done, value) = self.iterator_next(&iter)?;
                if is_done {
                    frame.pc = done as usize;
                } else {
                    frame.stack.push(value);
                }
            }
            Op::IterNextDestructure { temp, done } => {
                let iter = frame
                    .temps
                    .get(temp as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                let (is_done, value) = self.iterator_next(&iter)?;
                if is_done {
                    frame.stack.push(Value::Undefined);
                    frame.pc = done as usize;
                } else {
                    frame.stack.push(value);
                }
            }
            Op::IterRest { temp } => {
                let iter = frame
                    .temps
                    .get(temp as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                let mut vals = Vec::new();
                loop {
                    let (is_done, value) = self.iterator_next(&iter)?;
                    if is_done {
                        break;
                    }
                    vals.push(value);
                }
                let arr = self.create_array_from_list(vals)?;
                frame.stack.push(Value::Object(arr));
            }
            Op::IterClose { temp } => {
                let iter = frame
                    .temps
                    .get(temp as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                self.iterator_close(&iter, None)?;
                if frame.temps.len() > temp as usize {
                    frame.temps[temp as usize] = Value::Undefined;
                }
            }
            Op::ForInInit(temp) => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                let iter = self.create_for_in_iterator(&value)?;
                if frame.temps.len() <= temp as usize {
                    frame.temps.resize(temp as usize + 1, Value::Undefined);
                }
                frame.temps[temp as usize] = iter;
            }
            Op::ForInNext { temp, done } => {
                let iter = frame
                    .temps
                    .get(temp as usize)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                match self.for_in_next(&iter)? {
                    Some(key) => frame.stack.push(Value::String(key)),
                    None => frame.pc = done as usize,
                }
            }
            Op::CreateRegExp(idx) => {
                let re = self.constant_regexp(&frame.proto.constants[idx as usize])?;
                frame.stack.push(Value::Object(re));
            }
            Op::TemplateObject(idx) => {
                let obj = self.template_object(idx as usize, &frame.proto.constants)?;
                frame.stack.push(Value::Object(obj));
            }
            Op::SetNormalCompletion(target) => {
                frame.completion = None;
                frame.normal_target = Some(target);
            }
            Op::EndFinally => {
                let handler = frame.finally_handler.take();
                let completion = frame.completion.take();
                let normal_target = frame.normal_target.take();
                match completion {
                    None => {
                        if let Some(t) = normal_target {
                            frame.pc = t as usize;
                        }
                    }
                    Some(comp) => {
                        let outer = handler
                            .and_then(|h| frame.proto.handlers[h].outer)
                            .map(|h| h as usize);
                        let dispatch = self.dispatch_from_handler(frame, outer, comp)?;
                        if let Some(out) = dispatch {
                            return Ok(Some(out));
                        }
                    }
                }
            }
            Op::Yield => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                frame.finished = false;
                return Ok(Some(ExecOutcome::Yield(value)));
            }
            Op::Await => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                return Ok(Some(ExecOutcome::Await(value)));
            }
            Op::AsyncReturn => {
                let value = frame.stack.pop().unwrap_or(Value::Undefined);
                return Ok(Some(ExecOutcome::Return(value)));
            }
            Op::ThrowConstAssignment => {
                return self.throw_type_error("Assignment to constant variable.");
            }
        }
        Ok(None)
    }

    fn constant_to_value(&mut self, c: &Constant) -> JsResult<Value> {
        match c {
            Constant::Number(n) => Ok(Value::Number(*n)),
            Constant::String(s) => Ok(Value::String(s.clone())),
            Constant::BigInt(b) => Ok(Value::BigInt(b.clone())),
            Constant::Symbol(s) => Ok(Value::Symbol(s.clone())),
            Constant::RegExp { source, flags } => Ok(Value::Object(
                self.make_regexp(source.clone(), flags.clone())?,
            )),
            Constant::Template(_) => self.throw_syntax_error("Invalid template constant"),
            Constant::Class(_) => self.throw_syntax_error("Invalid class constant"),
        }
    }

    fn const_string(&self, c: &Constant) -> JsString {
        match c {
            Constant::String(s) => s.clone(),
            _ => JsString::empty(),
        }
    }

    fn const_key(&self, c: &Constant) -> PropertyKey {
        PropertyKey::String(self.const_string(c))
    }

    fn load_var(&mut self, frame: &Frame, depth: u16, slot: u16) -> JsResult<Value> {
        if let Some(env) = &frame.env {
            if let Some(e) = env.ancestor(depth as usize) {
                if let Some(v) = e.get(slot as usize) {
                    return Ok(v);
                }
                return self.throw_reference_error("Cannot access variable before initialization");
            }
        }
        self.throw_reference_error("Variable environment missing")
    }

    fn store_var(
        &mut self,
        frame: &Frame,
        depth: u16,
        slot: u16,
        value: Value,
        init: bool,
    ) -> JsResult<()> {
        if let Some(env) = &frame.env {
            if let Some(e) = env.ancestor(depth as usize) {
                if init {
                    e.initialize(slot as usize, value);
                    return Ok(());
                }
                if e.slot_is_initialized(slot as usize) {
                    e.set(slot as usize, value);
                    return Ok(());
                }
                return self.throw_reference_error("Cannot access variable before initialization");
            }
        }
        self.throw_reference_error("Variable environment missing")
    }

    fn load_global(&mut self, name: &JsString) -> JsResult<Value> {
        if let Some(v) = self.try_load_global(name)? {
            return Ok(v);
        }
        let mut msg = String::from("ReferenceError: ");
        msg.push_str(&name.to_string_lossy());
        msg.push_str(" is not defined");
        self.throw_reference_error(&msg)
    }

    pub fn try_load_global(&mut self, name: &JsString) -> JsResult<Option<Value>> {
        let key = PropertyKey::String(name.clone());
        if self.has_property(&self.realm.global.clone(), &key)? {
            return Ok(Some(self.get_property(&self.global_this(), &key)?));
        }
        Ok(None)
    }

    fn store_global(&mut self, name: &JsString, value: Value, strict: bool) -> JsResult<()> {
        let key = PropertyKey::String(name.clone());
        let global = self.realm.global.clone();
        if self.has_property(&global, &key)? {
            self.set_property(&self.global_this(), key, value, strict)?;
            return Ok(());
        }
        if strict {
            return self.throw_reference_error("Global variable is not defined");
        }
        self.create_data_property_or_throw(&global, key, value)?;
        Ok(())
    }

    fn declare_global_var(&mut self, name: &JsString) -> JsResult<()> {
        let global = self.realm.global.clone();
        let key = PropertyKey::String(name.clone());
        if !self.has_property(&global, &key)? {
            self.define_own_property(
                &global,
                key,
                Descriptor {
                    value: Some(Value::Undefined),
                    writable: Some(true),
                    enumerable: Some(true),
                    configurable: Some(false),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }

    fn declare_global_function(&mut self, name: &JsString, value: Value) -> JsResult<()> {
        let global = self.realm.global.clone();
        let key = PropertyKey::String(name.clone());
        self.define_own_property(
            &global,
            key,
            Descriptor {
                value: Some(value),
                writable: Some(true),
                enumerable: Some(true),
                configurable: Some(false),
                ..Default::default()
            },
        )?;
        Ok(())
    }

    fn delete_property_value(&mut self, obj: &Value, key: &PropertyKey) -> JsResult<bool> {
        match obj {
            Value::Object(o) => self.delete_property(o, key),
            Value::Undefined | Value::Null => {
                self.throw_type_error("Cannot convert undefined or null to object")
            }
            _ => {
                let o = self.to_object(obj)?;
                self.delete_property(&o, key)
            }
        }
    }

    fn make_closure(&mut self, proto: Arc<FunctionProto>, frame: &Frame) -> JsObject {
        let home = frame.home.clone();
        self.make_closure_with_home(proto, frame, home)
    }

    fn make_closure_with_home(
        &mut self,
        proto: Arc<FunctionProto>,
        frame: &Frame,
        home: Option<JsObject>,
    ) -> JsObject {
        let env = frame
            .env
            .clone()
            .unwrap_or_else(|| EnvRef::declarative(None, 0));
        let user = UserFunction {
            name: proto.name.clone(),
            proto: proto.clone(),
            env,
            home,
            lexical_this: if proto.is_arrow {
                Some(frame.this.clone())
            } else {
                None
            },
            lexical_new_target: if proto.is_arrow {
                frame.new_target.clone()
            } else {
                None
            },
            class_fields: RefCell::new(Vec::new()),
        };
        let f = JsObject::user_function(
            Some(self.realm.intrinsics.function_prototype.clone()),
            proto.name.clone(),
            proto.length,
            user,
        );
        if proto.kind.is_constructor()
            && !proto.is_arrow
            && !matches!(
                proto.kind,
                ProtoKind::ClassConstructor | ProtoKind::DerivedConstructor
            )
        {
            self.create_function_prototype_property(&f);
        } else if matches!(proto.kind, ProtoKind::Generator | ProtoKind::AsyncGenerator) {
            let base = if matches!(proto.kind, ProtoKind::AsyncGenerator) {
                self.realm.intrinsics.async_generator_prototype.clone()
            } else {
                self.realm.intrinsics.generator_prototype.clone()
            };
            let proto_obj = JsObject::ordinary(Some(base));
            f.define_fixed("prototype", Value::Object(proto_obj));
        }
        f
    }

    pub fn create_function_prototype_property(&mut self, f: &JsObject) {
        let proto_obj = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        proto_obj.put_hidden("constructor", Value::Object(f.clone()));
        f.define_fixed("prototype", Value::Object(proto_obj));
    }

    fn get_super_property(&mut self, frame: &Frame, key: &PropertyKey) -> JsResult<Value> {
        let home = frame.home.clone();
        let proto = match home {
            Some(h) => h.0.borrow().prototype.clone(),
            None => None,
        };
        match proto {
            Some(p) => self.get_property_with_receiver(&p, key, &frame.this),
            None => Ok(Value::Undefined),
        }
    }

    fn set_super_property(
        &mut self,
        frame: &Frame,
        key: PropertyKey,
        value: Value,
    ) -> JsResult<()> {
        let home = frame.home.clone();
        let proto = match home {
            Some(h) => h.0.borrow().prototype.clone(),
            None => None,
        };
        if let Some(p) = proto {
            self.set_property_object(&p, key, value, &frame.this, frame.strict)?;
        }
        Ok(())
    }

    fn super_construct(&mut self, frame: &mut Frame, args: &[Value]) -> JsResult<Value> {
        let parent = frame
            .func_obj
            .as_ref()
            .and_then(|f| f.0.borrow().prototype.clone());
        let parent = match parent {
            Some(p) => p,
            None => return self.throw_type_error("Super constructor is null"),
        };
        let new_target = frame.new_target.clone();
        let result = self.construct_function(&Value::Object(parent), args, new_target.as_ref())?;
        frame.this = result.clone();
        if let Some(user) = frame.func.clone() {
            self.initialize_class_fields(&user, &result)?;
        }
        Ok(result)
    }

    fn initialize_class_fields(&mut self, user: &Rc<UserFunction>, this: &Value) -> JsResult<()> {
        let fields = user.class_fields.borrow().clone();
        for field in fields {
            if field.is_static {
                continue;
            }
            let value = match &field.initializer {
                Some(f) => self.call_function(&Value::Object(f.clone()), this, &[])?,
                None => Value::Undefined,
            };
            if let Value::Object(o) = this {
                self.create_data_property_or_throw(o, field.key.clone(), value)?;
            }
        }
        Ok(())
    }
    pub fn iterable_to_list(&mut self, value: &Value) -> JsResult<Vec<Value>> {
        let iter = self.get_iterator(value)?;
        let mut out = Vec::new();
        loop {
            let (done, v) = self.iterator_next(&iter)?;
            if done {
                break;
            }
            out.push(v);
        }
        Ok(out)
    }

    fn create_for_in_iterator(&mut self, value: &Value) -> JsResult<Value> {
        let mut keys = Vec::new();
        if !value.is_nullish() {
            let obj = self.to_object(value)?;
            let mut cur = Some(obj);
            let mut seen = std::collections::HashSet::new();
            while let Some(o) = cur {
                let own = self.own_property_keys(&o)?;
                for k in own {
                    if let PropertyKey::String(s) = &k {
                        if seen.insert(s.clone()) {
                            keys.push(k);
                        }
                    }
                }
                cur = o.0.borrow().prototype.clone();
            }
        }
        let record = JsObject::new(ObjectData {
            prototype: Some(self.realm.intrinsics.iterator_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Iterator(IteratorData {
                iterable: value.clone(),
                next_method: Value::Undefined,
                done: false,
            }),
            private: IndexMap::new(),
        });
        record.put_hidden("__keys", Value::Number(keys.len() as f64));
        for (i, k) in keys.into_iter().enumerate() {
            let key = PropertyKey::String(JsString::from_str(&format!("__k{}", i)));
            record.put_hidden(key, k.to_value());
        }
        Ok(Value::Object(record))
    }

    fn for_in_next(&mut self, iter: &Value) -> JsResult<Option<JsString>> {
        let obj = match iter.as_object_cloned() {
            Some(o) => o,
            None => return Ok(None),
        };
        let idx = obj
            .get_hidden("__index")
            .and_then(|v| v.as_number())
            .unwrap_or(0.0) as usize;
        let total = obj
            .get_hidden("__keys")
            .and_then(|v| v.as_number())
            .unwrap_or(0.0) as usize;
        if idx >= total {
            return Ok(None);
        }
        obj.put_hidden("__index", Value::Number((idx + 1) as f64));
        let key = obj
            .get_hidden(&format!("__k{}", idx))
            .and_then(|v| v.as_string().cloned());
        Ok(key)
    }

    fn binary_op(&mut self, op: BinOp, left: &Value, right: &Value) -> JsResult<Value> {
        match op {
            BinOp::Add => self.add_values(left, right),
            BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod
            | BinOp::Pow
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::UShr
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor => self.numeric_op(op, left, right),
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let result = self.compare_values(op, left, right, true)?;
                Ok(result.map(Value::Bool).unwrap_or(Value::Bool(false)))
            }
            BinOp::Eq => Ok(Value::Bool(self.abstract_equals(left, right)?)),
            BinOp::Ne => Ok(Value::Bool(!self.abstract_equals(left, right)?)),
            BinOp::StrictEq => Ok(Value::Bool(Value::strict_equals(left, right))),
            BinOp::StrictNe => Ok(Value::Bool(!Value::strict_equals(left, right))),
            BinOp::In => {
                if !right.is_object() {
                    return self.throw_type_error("Right-hand side of 'in' is not an object");
                }
                let key = self.to_property_key(left)?;
                let has = self.has_property_value(right, &key)?;
                Ok(Value::Bool(has))
            }
            BinOp::InstanceOf => Ok(Value::Bool(self.instanceof(left, right)?)),
        }
    }

    fn unary_op(&mut self, op: UnaryOp, value: &Value) -> JsResult<Value> {
        match op {
            UnaryOp::Plus => Ok(Value::Number(self.to_number(value)?)),
            UnaryOp::Minus => {
                let n = self.to_numeric(value)?;
                match n {
                    Value::BigInt(b) => Ok(Value::BigInt(Rc::new(-(*b).clone()))),
                    Value::Number(n) => Ok(Value::Number(-n)),
                    _ => Ok(Value::Number(f64::NAN)),
                }
            }
            UnaryOp::Not => Ok(Value::Bool(!value.truthy())),
            UnaryOp::BitNot => {
                if value.is_bigint() {
                    let b = self.to_bigint(value)?;
                    Ok(Value::BigInt(Rc::new(!(*b).clone())))
                } else {
                    let n = self.to_int32(value)?;
                    Ok(Value::Number(!n as f64))
                }
            }
            UnaryOp::TypeOf => Ok(Value::String(JsString::from_str(value.type_of()))),
            UnaryOp::Void => Ok(Value::Undefined),
            UnaryOp::Delete => Ok(Value::Bool(true)),
            UnaryOp::Increment | UnaryOp::Decrement => {
                let n = self.to_numeric(value)?;
                match n {
                    Value::BigInt(b) => {
                        let one = num_bigint::BigInt::from(1);
                        let r = if matches!(op, UnaryOp::Increment) {
                            (*b).clone() + one
                        } else {
                            (*b).clone() - one
                        };
                        Ok(Value::BigInt(Rc::new(r)))
                    }
                    Value::Number(n) => Ok(Value::Number(if matches!(op, UnaryOp::Increment) {
                        n + 1.0
                    } else {
                        n - 1.0
                    })),
                    _ => Ok(Value::Number(f64::NAN)),
                }
            }
        }
    }

    fn add_values(&mut self, left: &Value, right: &Value) -> JsResult<Value> {
        let lp = self.to_primitive(left, "default")?;
        let rp = self.to_primitive(right, "default")?;
        if lp.is_string() || rp.is_string() {
            let l = self.to_string(&lp)?;
            let r = self.to_string(&rp)?;
            return Ok(Value::String(l.concat(&r)));
        }
        let l = self.to_numeric(&lp)?;
        let r = self.to_numeric(&rp)?;
        match (l, r) {
            (Value::BigInt(a), Value::BigInt(b)) => Ok(Value::BigInt(Rc::new((*a).clone() + &*b))),
            (Value::BigInt(_), _) | (_, Value::BigInt(_)) => {
                self.throw_type_error("Cannot mix BigInt and other types")
            }
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            _ => Ok(Value::Number(f64::NAN)),
        }
    }

    fn numeric_op(&mut self, op: BinOp, left: &Value, right: &Value) -> JsResult<Value> {
        let l = self.to_numeric(left)?;
        let r = self.to_numeric(right)?;
        if let (Value::BigInt(a), Value::BigInt(b)) = (&l, &r) {
            use num_traits::{Signed, Zero};
            let a = (**a).clone();
            let b = (**b).clone();
            return match op {
                BinOp::Sub => Ok(Value::BigInt(Rc::new(a - b))),
                BinOp::Mul => Ok(Value::BigInt(Rc::new(a * b))),
                BinOp::Div => {
                    if b.is_zero() {
                        self.throw_range_error("Division by zero")
                    } else {
                        Ok(Value::BigInt(Rc::new(a / b)))
                    }
                }
                BinOp::Mod => {
                    if b.is_zero() {
                        self.throw_range_error("Division by zero")
                    } else {
                        Ok(Value::BigInt(Rc::new(a % b)))
                    }
                }
                BinOp::Pow => {
                    if b.is_negative() {
                        self.throw_range_error("Exponent must be positive")
                    } else {
                        let exp = b.to_u32().unwrap_or(0);
                        Ok(Value::BigInt(Rc::new(num_traits::Pow::pow(&a, exp))))
                    }
                }
                BinOp::Shl | BinOp::Shr | BinOp::UShr => {
                    if b.is_negative() {
                        return self.throw_range_error("BigInt shift negative");
                    }
                    let shift = b.to_usize().unwrap_or(usize::MAX);
                    if shift > 1_000_000 {
                        return self.throw_range_error("BigInt shift too large");
                    }
                    match op {
                        BinOp::Shl => Ok(Value::BigInt(Rc::new(a << shift))),
                        BinOp::Shr => Ok(Value::BigInt(Rc::new(a >> shift))),
                        _ => self.throw_type_error("BigInts have no unsigned right shift"),
                    }
                }
                BinOp::BitAnd => Ok(Value::BigInt(Rc::new(a & b))),
                BinOp::BitOr => Ok(Value::BigInt(Rc::new(a | b))),
                BinOp::BitXor => Ok(Value::BigInt(Rc::new(a ^ b))),
                _ => self.throw_type_error("Unsupported BigInt operation"),
            };
        }
        if l.is_bigint() || r.is_bigint() {
            return self.throw_type_error("Cannot mix BigInt and other types");
        }
        let a = self.to_number(&l)?;
        let b = self.to_number(&r)?;
        let result = match op {
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
            BinOp::Mod => a % b,
            BinOp::Pow => a.powf(b),
            BinOp::Shl => (f64_to_int32_bits(a) << (f64_to_uint32(b) & 31)) as f64,
            BinOp::Shr => (f64_to_int32_bits(a) >> (f64_to_uint32(b) & 31)) as f64,
            BinOp::UShr => ((f64_to_uint32(a) as i64) >> (f64_to_uint32(b) & 31)) as u32 as f64,
            BinOp::BitAnd => (f64_to_int32_bits(a) & f64_to_int32_bits(b)) as f64,
            BinOp::BitOr => (f64_to_int32_bits(a) | f64_to_int32_bits(b)) as f64,
            BinOp::BitXor => (f64_to_int32_bits(a) ^ f64_to_int32_bits(b)) as f64,
            _ => f64::NAN,
        };
        Ok(Value::Number(result))
    }

    fn compare_values(
        &mut self,
        op: BinOp,
        left: &Value,
        right: &Value,
        allow_undefined: bool,
    ) -> JsResult<Option<bool>> {
        let lp = self.to_primitive(left, "number")?;
        let rp = self.to_primitive(right, "number")?;
        if lp.is_string() && rp.is_string() {
            let a = self.to_string(&lp)?;
            let b = self.to_string(&rp)?;
            let cmp = a.as_slice().cmp(b.as_slice());
            return Ok(Some(match op {
                BinOp::Lt => cmp == std::cmp::Ordering::Less,
                BinOp::Gt => cmp == std::cmp::Ordering::Greater,
                BinOp::Le => cmp != std::cmp::Ordering::Greater,
                BinOp::Ge => cmp != std::cmp::Ordering::Less,
                _ => false,
            }));
        }
        if lp.is_bigint() && rp.is_bigint() {
            let a = self.to_bigint(&lp)?;
            let b = self.to_bigint(&rp)?;
            return Ok(Some(match op {
                BinOp::Lt => *a < *b,
                BinOp::Gt => *a > *b,
                BinOp::Le => *a <= *b,
                BinOp::Ge => *a >= *b,
                _ => false,
            }));
        }
        let a = self.to_number(&lp)?;
        let b = self.to_number(&rp)?;
        if a.is_nan() || b.is_nan() {
            return Ok(if allow_undefined { None } else { Some(false) });
        }
        Ok(Some(match op {
            BinOp::Lt => a < b,
            BinOp::Gt => a > b,
            BinOp::Le => a <= b,
            BinOp::Ge => a >= b,
            _ => false,
        }))
    }

    pub fn abstract_equals(&mut self, left: &Value, right: &Value) -> JsResult<bool> {
        if std::mem::discriminant(left) == std::mem::discriminant(right) {
            return Ok(Value::strict_equals(left, right));
        }
        match (left, right) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => Ok(true),
            (Value::Number(_), Value::String(_)) => {
                let r = self.to_number(right)?;
                Ok(self.to_number(left)? == r)
            }
            (Value::String(_), Value::Number(_)) => {
                let l = self.to_number(left)?;
                Ok(l == self.to_number(right)?)
            }
            (Value::BigInt(_), Value::String(_)) => {
                let b = string_to_bigint(right.as_string().unwrap());
                Ok(
                    b.map(|b| left.as_bigint().map(|a| **a == b).unwrap_or(false))
                        .unwrap_or(false),
                )
            }
            (Value::String(_), Value::BigInt(_)) => self.abstract_equals(right, left),
            (Value::Bool(_), _) => {
                let l = Value::Number(self.to_number(left)?);
                self.abstract_equals(&l, right)
            }
            (_, Value::Bool(_)) => {
                let r = Value::Number(self.to_number(right)?);
                self.abstract_equals(left, &r)
            }
            (Value::Number(_), Value::BigInt(_)) | (Value::BigInt(_), Value::Number(_)) => {
                let (n, b) = if left.is_number() {
                    (left.as_number().unwrap(), right.as_bigint().unwrap())
                } else {
                    (right.as_number().unwrap(), left.as_bigint().unwrap())
                };
                if !n.is_finite() || n.fract() != 0.0 {
                    return Ok(false);
                }
                Ok(b.to_f64() == Some(n))
            }
            (Value::Object(_), _) => {
                let prim = self.to_primitive(left, "default")?;
                self.abstract_equals(&prim, right)
            }
            (_, Value::Object(_)) => {
                let prim = self.to_primitive(right, "default")?;
                self.abstract_equals(left, &prim)
            }
            _ => Ok(false),
        }
    }

    pub fn instanceof(&mut self, left: &Value, right: &Value) -> JsResult<bool> {
        let target = match right.as_object_cloned() {
            Some(o) if o.is_callable() => o,
            _ => return self.throw_type_error("Right-hand side of 'instanceof' is not callable"),
        };
        let has_instance = self.get_property(
            &Value::Object(target.clone()),
            &PropertyKey::Symbol(self.realm.intrinsics.symbol_has_instance.clone()),
        )?;
        if let Some(f) = has_instance.as_object_cloned() {
            if f.is_callable() {
                let result =
                    self.call_function(&Value::Object(f), &Value::Object(target), &[left.clone()])?;
                return Ok(result.truthy());
            }
        }
        if !target.is_constructor() {
            return self.throw_type_error("Right-hand side of 'instanceof' is not callable");
        }
        self.ordinary_has_instance(&target, left)
    }

    fn ordinary_has_instance(&mut self, ctor: &JsObject, value: &Value) -> JsResult<bool> {
        let mut obj = match value.as_object_cloned() {
            Some(o) => o,
            None => return Ok(false),
        };
        let proto = self.get_property(
            &Value::Object(ctor.clone()),
            &PropertyKey::from_str("prototype"),
        )?;
        let proto = match proto.as_object_cloned() {
            Some(p) => p,
            None => {
                return self.throw_type_error("Function has non-object prototype in instanceof")
            }
        };
        loop {
            let p = obj.0.borrow().prototype.clone();
            match p {
                Some(p) => {
                    if p.ptr_eq(&proto) {
                        return Ok(true);
                    }
                    obj = p;
                }
                None => return Ok(false),
            }
        }
    }

    pub fn create_arguments_object(&mut self, frame: &Frame) -> JsResult<Value> {
        let obj = JsObject::new(ObjectData {
            prototype: Some(self.realm.intrinsics.object_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Arguments(ArgumentsData {
                mapped: Vec::new(),
                parameter_map_len: 0,
            }),
            private: IndexMap::new(),
        });
        obj.put_hidden("length", Value::Number(frame.args.len() as f64));
        for (i, a) in frame.args.iter().enumerate() {
            obj.put_hidden(
                PropertyKey::String(JsString::from_str(&i.to_string())),
                a.clone(),
            );
        }
        Ok(Value::Object(obj))
    }

    pub fn make_regexp(&mut self, source: JsString, flags: JsString) -> JsResult<JsObject> {
        let source_str = source.to_string_lossy();
        let flags_str = flags.to_string_lossy();
        // Validate flags.
        let mut seen = std::collections::HashSet::new();
        for c in flags_str.chars() {
            if !"dgimsuvy".contains(c) || !seen.insert(c) {
                return self.throw_syntax_error("Invalid regular expression flags");
            }
        }
        if seen.contains(&'u') && seen.contains(&'v') {
            return self.throw_syntax_error("Invalid regular expression flags");
        }
        let mut regress_flags = String::new();
        for c in flags_str.chars() {
            match c {
                'i' | 'm' | 's' | 'y' => regress_flags.push(c),
                'u' | 'v' => {
                    if !regress_flags.contains('u') {
                        regress_flags.push('u');
                    }
                }
                _ => {}
            }
        }
        let cache_key = JsString::from_str(&format!("{}\u{0}{}", source_str, regress_flags));
        let regex = if let Some(cached) = self.realm.regexp_cache.get(&cache_key) {
            cached.clone()
        } else {
            let compiled = regress::Regex::with_flags(&source_str, regress_flags.as_str())
                .ok()
                .map(Rc::new);
            self.realm.regexp_cache.insert(cache_key, compiled.clone());
            compiled
        };
        if regex.is_none() {
            return self.throw_syntax_error("Invalid regular expression");
        }
        let obj = JsObject::new(ObjectData {
            prototype: Some(self.realm.intrinsics.regexp_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::RegExp(RegExpData {
                source: source.clone(),
                flags: flags.clone(),
                regex,
                last_index: 0.0,
            }),
            private: IndexMap::new(),
        });
        obj.put_hidden("lastIndex", Value::Number(0.0));
        Ok(obj)
    }

    fn constant_regexp(&mut self, c: &Constant) -> JsResult<JsObject> {
        match c {
            Constant::RegExp { source, flags } => self.make_regexp(source.clone(), flags.clone()),
            _ => self.throw_syntax_error("Invalid regexp constant"),
        }
    }

    fn template_object(&mut self, idx: usize, constants: &[Constant]) -> JsResult<JsObject> {
        if let Some(Some(obj)) = self.realm.template_cache.get(idx) {
            return Ok(obj.clone());
        }
        let site = match &constants[idx] {
            Constant::Template(t) => t,
            _ => return self.throw_syntax_error("Invalid template site"),
        };
        let arr = JsObject::array(Some(self.realm.intrinsics.array_prototype.clone()));
        for (i, cooked) in site.cooked.iter().enumerate() {
            let v = cooked
                .as_ref()
                .map(|s| Value::String(s.clone()))
                .unwrap_or(Value::Undefined);
            arr.put(PropertyKey::String(JsString::from_str(&i.to_string())), v);
        }
        if let ObjectKind::Array(a) = &mut arr.0.borrow_mut().kind {
            a.length = site.cooked.len() as u32;
        }
        let raw = JsObject::array(Some(self.realm.intrinsics.array_prototype.clone()));
        for (i, r) in site.raw.iter().enumerate() {
            raw.put(
                PropertyKey::String(JsString::from_str(&i.to_string())),
                Value::String(r.clone()),
            );
        }
        if let ObjectKind::Array(a) = &mut raw.0.borrow_mut().kind {
            a.length = site.raw.len() as u32;
        }
        arr.define_fixed("raw", Value::Object(raw));
        arr.0.borrow_mut().extensible = false;
        if self.realm.template_cache.len() <= idx {
            self.realm.template_cache.resize(idx + 1, None);
        }
        self.realm.template_cache[idx] = Some(arr.clone());
        Ok(arr)
    }

    pub fn get_property_with_receiver(
        &mut self,
        obj: &JsObject,
        key: &PropertyKey,
        receiver: &Value,
    ) -> JsResult<Value> {
        let mut cur = obj.clone();
        loop {
            let own = cur.0.borrow().get_own_property(key);
            if let Some(p) = own {
                return match p.value {
                    PropertyValue::Data(v) => Ok(v),
                    PropertyValue::Accessor { get, .. } => match get {
                        Some(g) => self.call_function(&Value::Object(g), receiver, &[]),
                        None => Ok(Value::Undefined),
                    },
                };
            }
            let proto = cur.0.borrow().prototype.clone();
            match proto {
                Some(p) => cur = p,
                None => return Ok(Value::Undefined),
            }
        }
    }
}

pub fn f64_to_int32_bits(n: f64) -> i32 {
    crate::ops::f64_to_int32(n)
}

#[derive(Clone)]
pub enum Resume {
    Normal,
    Value(Value),
    Throw(Value),
    Return(Value),
}

pub enum ExecOutcome {
    Return(Value),
    Yield(Value),
    Await(Value),
    Throw(Value),
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub proto: Arc<FunctionProto>,
    pub func: Option<Rc<UserFunction>>,
    pub func_obj: Option<JsObject>,
    pub env: Option<EnvRef>,
    pub env_depth: u32,
    pub stack: Vec<Value>,
    pub temps: Vec<Value>,
    pub pc: usize,
    pub this: Value,
    pub new_target: Option<JsObject>,
    pub home: Option<JsObject>,
    pub args: Vec<Value>,
    pub completion: Option<Completion>,
    pub normal_target: Option<u32>,
    pub finally_handler: Option<usize>,
    pub strict: bool,
    pub finished: bool,
}

impl Frame {
    fn restore_env(&mut self, depth: u32) {
        if self.env_depth <= depth {
            return;
        }
        let mut env = self.env.clone();
        for _ in 0..(self.env_depth - depth) {
            env = env.and_then(|e| e.parent());
        }
        self.env = env;
        self.env_depth = depth;
    }
}

impl Interp {
    fn dispatch_completion(
        &mut self,
        frame: &mut Frame,
        completion: Completion,
    ) -> Result<Option<ExecOutcome>, Completion> {
        let pc = frame.pc.saturating_sub(1) as u32;
        let mut best: Option<usize> = None;
        for (i, h) in frame.proto.handlers.iter().enumerate() {
            if h.start <= pc && pc < h.end {
                if best
                    .map(|b| h.start > frame.proto.handlers[b].start)
                    .unwrap_or(true)
                {
                    best = Some(i);
                }
            }
        }
        self.dispatch_from_handler(frame, best, completion)
    }

    fn dispatch_from_handler(
        &mut self,
        frame: &mut Frame,
        handler: Option<usize>,
        mut completion: Completion,
    ) -> Result<Option<ExecOutcome>, Completion> {
        let mut current = handler;
        while let Some(idx) = current {
            let h = frame.proto.handlers[idx].clone();
            current = h.outer.map(|i| i as usize);
            if let (Completion::Throw(v), Some(catch)) = (&completion, h.catch_target) {
                frame.restore_env(h.env_depth);
                frame.stack.truncate(h.stack_height as usize);
                frame.completion = None;
                frame.finally_handler = None;
                frame.normal_target = None;
                frame.stack.push(v.clone());
                frame.pc = catch as usize;
                return Ok(None);
            }
            if let Some(finally) = h.finally_target {
                frame.restore_env(h.env_depth);
                frame.stack.truncate(h.stack_height as usize);
                frame.completion = Some(completion);
                frame.finally_handler = Some(idx);
                frame.normal_target = None;
                frame.pc = finally as usize;
                return Ok(None);
            }
            match completion {
                Completion::Normal => return Ok(None),
                _ => {}
            }
        }
        match completion {
            Completion::Throw(v) => Ok(Some(ExecOutcome::Throw(v))),
            Completion::Return(v) => Ok(Some(ExecOutcome::Return(v))),
            Completion::Break(_) | Completion::Continue(_) => {
                Err(Completion::Throw(self.realm.realm_error(
                    &self.realm.intrinsics.syntax_error_prototype.clone(),
                    "Illegal break/continue statement",
                )))
            }
            Completion::Normal => Ok(None),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratorData {
    pub state: GeneratorState,
    pub frame: Option<Frame>,
    pub this: Value,
    pub done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorState {
    SuspendedStart,
    SuspendedYield,
    Executing,
    Completed,
}

#[derive(Clone, Debug)]
pub struct AsyncGeneratorData {
    pub state: GeneratorState,
    pub frame: Option<Frame>,
    pub done: bool,
    pub queue: std::collections::VecDeque<AsyncGenRequest>,
    pub pending: Option<AsyncGenRequest>,
    pub running: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncGenRequestKind {
    Next,
    Throw,
    Return,
}

#[derive(Clone, Debug)]
pub struct AsyncGenRequest {
    pub kind: AsyncGenRequestKind,
    pub value: Value,
    pub cap: PromiseCapability,
}

pub struct AsyncState {
    pub frame: RefCell<Option<Frame>>,
    pub resolve: JsObject,
    pub reject: JsObject,
}
