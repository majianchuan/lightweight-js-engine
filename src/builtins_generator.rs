//! Generator functions and generator objects.

use crate::builtins::native_fn;
use crate::env::EnvRef;
use crate::object::*;
use crate::value::*;
use crate::vm::{Frame, GeneratorData, GeneratorState, Interp, Resume};
use std::rc::Rc;

impl Interp {
    pub fn install_generator(&mut self) {
        let proto = self.realm.intrinsics.generator_prototype.clone();
        self.define_method(&proto, "next", 1, |interp, this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.generator_resume(this, ResumeKind::Next, v)
        });
        self.define_method(&proto, "return", 1, |interp, this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.generator_resume(this, ResumeKind::Return, v)
        });
        self.define_method(&proto, "throw", 1, |interp, this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.generator_resume(this, ResumeKind::Throw, v)
        });
        let iter_this = self.native_method("[Symbol.iterator]", 0, |_interp, this, _args| {
            Ok(this.clone())
        });
        let desc = Descriptor {
            value: Some(Value::Object(iter_this)),
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
            value: Some(Value::String(JsString::from_str("Generator"))),
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

    pub fn create_generator(
        &mut self,
        user: Rc<UserFunction>,
        this: Value,
        args: &[Value],
        func_obj: &JsObject,
    ) -> JsResult<Value> {
        let proto = user.proto.clone();
        let env = EnvRef::declarative(Some(user.env.clone()), proto.env_slots as usize);
        let mut this = this;
        if proto.is_arrow {
            this = user.lexical_this.clone().unwrap_or(Value::Undefined);
        } else if !proto.is_strict && this.is_nullish() {
            this = self.global_this();
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
            new_target: None,
            home: user.home.clone(),
            args: args.to_vec(),
            completion: None,
            normal_target: None,
            finally_handler: None,
            strict: user.proto.is_strict,
            finished: false,
        };
        let inst_proto = self
            .get_property(
                &Value::Object(func_obj.clone()),
                &PropertyKey::from_str("prototype"),
            )
            .ok()
            .and_then(|v| v.as_object_cloned())
            .unwrap_or_else(|| self.realm.intrinsics.generator_prototype.clone());
        let obj = JsObject::new(ObjectData {
            prototype: Some(inst_proto),
            extensible: true,
            properties: indexmap::IndexMap::new(),
            kind: ObjectKind::Generator(GeneratorData {
                state: GeneratorState::SuspendedStart,
                frame: Some(frame),
                this,
                done: false,
            }),
            private: indexmap::IndexMap::new(),
        });
        Ok(Value::Object(obj))
    }

    fn generator_resume(&mut self, this: &Value, kind: ResumeKind, arg: Value) -> JsResult<Value> {
        let obj = match this.as_object_cloned() {
            Some(o) if matches!(&o.borrow().kind, ObjectKind::Generator(_)) => o,
            _ => return self.throw_type_error("Generator method called on incompatible receiver"),
        };
        let (state, done) = {
            let data = obj.borrow();
            if let ObjectKind::Generator(g) = &data.kind {
                (g.state, g.done)
            } else {
                return self.throw_type_error("Generator method called on incompatible receiver");
            }
        };
        if done || state == GeneratorState::Completed {
            return match kind {
                ResumeKind::Next => self.iterator_result(Value::Undefined, true),
                ResumeKind::Return => self.iterator_result(arg, true),
                ResumeKind::Throw => Err(Completion::Throw(arg)),
            };
        }
        if state == GeneratorState::Executing {
            return self.throw_type_error("Generator is already running");
        }
        let frame = {
            let mut data = obj.borrow_mut();
            if let ObjectKind::Generator(g) = &mut data.kind {
                g.state = GeneratorState::Executing;
                g.frame.take()
            } else {
                None
            }
        };
        let frame = match frame {
            Some(f) => f,
            None => return self.throw_type_error("Generator frame is missing"),
        };
        let resume = match kind {
            ResumeKind::Next => {
                if state == GeneratorState::SuspendedStart {
                    Resume::Normal
                } else {
                    Resume::Value(arg)
                }
            }
            ResumeKind::Return => Resume::Return(arg),
            ResumeKind::Throw => Resume::Throw(arg),
        };
        let outcome = self.execute_frame(frame, resume);
        let returned_frame = self.last_frame.take();
        match outcome {
            Ok(crate::vm::ExecOutcome::Return(v)) => {
                self.set_generator_state(&obj, GeneratorState::Completed, true, None);
                self.iterator_result(v, true)
            }
            Ok(crate::vm::ExecOutcome::Yield(v)) => {
                self.set_generator_state(
                    &obj,
                    GeneratorState::SuspendedYield,
                    false,
                    returned_frame,
                );
                self.iterator_result(v, false)
            }
            Ok(crate::vm::ExecOutcome::Throw(v)) => {
                self.set_generator_state(&obj, GeneratorState::Completed, true, None);
                Err(Completion::Throw(v))
            }
            Ok(crate::vm::ExecOutcome::Await(_)) => {
                self.set_generator_state(&obj, GeneratorState::Completed, true, None);
                self.throw_type_error("Unexpected await in generator")
            }
            Err(c) => {
                self.set_generator_state(&obj, GeneratorState::Completed, true, None);
                Err(c)
            }
        }
    }

    fn set_generator_state(
        &mut self,
        obj: &JsObject,
        state: GeneratorState,
        done: bool,
        frame: Option<Frame>,
    ) {
        if let ObjectKind::Generator(g) = &mut obj.0.borrow_mut().kind {
            g.state = state;
            g.done = done;
            g.frame = frame;
        }
    }

    pub fn iterator_result(&mut self, value: Value, done: bool) -> JsResult<Value> {
        let obj = JsObject::ordinary(Some(self.realm.intrinsics.object_prototype.clone()));
        obj.put_hidden("value", value);
        obj.put_hidden("done", Value::Bool(done));
        Ok(Value::Object(obj))
    }
}

#[derive(Clone, Copy)]
enum ResumeKind {
    Next,
    Return,
    Throw,
}
