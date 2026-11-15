//! Async generator support.

use crate::builtins::native_fn;
use crate::env::EnvRef;
use crate::object::*;
use crate::value::*;
use crate::vm::{
    AsyncGenRequest, AsyncGenRequestKind, AsyncGeneratorData, ExecOutcome, Frame, GeneratorState,
    Interp, Resume,
};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

impl Interp {
    pub fn install_async_generator(&mut self) {
        let proto = self.realm.intrinsics.async_generator_prototype.clone();
        self.define_method(&proto, "next", 1, |interp, this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.async_generator_request(this, AsyncGenRequestKind::Next, v)
        });
        self.define_method(&proto, "return", 1, |interp, this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.async_generator_request(this, AsyncGenRequestKind::Return, v)
        });
        self.define_method(&proto, "throw", 1, |interp, this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.async_generator_request(this, AsyncGenRequestKind::Throw, v)
        });
        let iter = self.native_method("[Symbol.asyncIterator]", 0, |_interp, this, _args| {
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
            PropertyKey::Symbol(self.realm.intrinsics.symbol_async_iterator.clone()),
            desc,
        );
        let tag = Descriptor {
            value: Some(Value::String(JsString::from_str("AsyncGenerator"))),
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

    pub fn create_async_generator(
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
            this,
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
            .unwrap_or_else(|| self.realm.intrinsics.async_generator_prototype.clone());
        let obj = JsObject::new(ObjectData {
            prototype: Some(inst_proto),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::AsyncGenerator(AsyncGeneratorData {
                state: GeneratorState::SuspendedStart,
                frame: Some(frame),
                done: false,
                queue: VecDeque::new(),
                pending: None,
                running: false,
            }),
            private: IndexMap::new(),
        });
        Ok(Value::Object(obj))
    }

    fn async_generator_request(
        &mut self,
        this: &Value,
        kind: AsyncGenRequestKind,
        value: Value,
    ) -> JsResult<Value> {
        let gen = match this.as_object_cloned() {
            Some(o) if matches!(&o.borrow().kind, ObjectKind::AsyncGenerator(_)) => o,
            _ => {
                return self
                    .throw_type_error("AsyncGenerator method called on incompatible receiver")
            }
        };
        let cap = self.new_promise_capability()?;
        {
            let mut data = gen.0.borrow_mut();
            if let ObjectKind::AsyncGenerator(g) = &mut data.kind {
                g.queue.push_back(AsyncGenRequest {
                    kind,
                    value,
                    cap: cap.clone(),
                });
            }
        }
        self.async_generator_process(&gen)?;
        Ok(Value::Object(cap.promise))
    }

    fn async_generator_process(&mut self, gen: &JsObject) -> JsResult<()> {
        loop {
            let (running, done, has_req) = {
                let data = gen.borrow();
                if let ObjectKind::AsyncGenerator(g) = &data.kind {
                    (g.running, g.done, !g.queue.is_empty())
                } else {
                    return Ok(());
                }
            };
            if running || !has_req {
                return Ok(());
            }
            if done {
                let req = {
                    let mut data = gen.0.borrow_mut();
                    if let ObjectKind::AsyncGenerator(g) = &mut data.kind {
                        g.queue.pop_front()
                    } else {
                        None
                    }
                };
                if let Some(req) = req {
                    let value = if req.kind == AsyncGenRequestKind::Return {
                        req.value
                    } else {
                        Value::Undefined
                    };
                    let result = self.iterator_result(value, true)?;
                    let resolve = Value::Object(req.cap.resolve.clone());
                    let _ = self.call_function(&resolve, &Value::Undefined, &[result]);
                }
                continue;
            }
            let (req, frame) = {
                let mut data = gen.0.borrow_mut();
                if let ObjectKind::AsyncGenerator(g) = &mut data.kind {
                    let req = g.queue.pop_front();
                    let frame = g.frame.take();
                    g.running = true;
                    (req, frame)
                } else {
                    (None, None)
                }
            };
            let req = match req {
                Some(r) => r,
                None => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.running = false;
                    }
                    return Ok(());
                }
            };
            let frame = match frame {
                Some(f) => f,
                None => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.running = false;
                        g.done = true;
                    }
                    let result = self.iterator_result(Value::Undefined, true)?;
                    let resolve = Value::Object(req.cap.resolve.clone());
                    let _ = self.call_function(&resolve, &Value::Undefined, &[result]);
                    continue;
                }
            };
            let resume = match req.kind {
                AsyncGenRequestKind::Next => {
                    let st = {
                        let data = gen.borrow();
                        if let ObjectKind::AsyncGenerator(g) = &data.kind {
                            g.state
                        } else {
                            GeneratorState::SuspendedStart
                        }
                    };
                    if st == GeneratorState::SuspendedStart {
                        Resume::Normal
                    } else {
                        Resume::Value(req.value.clone())
                    }
                }
                AsyncGenRequestKind::Throw => Resume::Throw(req.value.clone()),
                AsyncGenRequestKind::Return => Resume::Return(req.value.clone()),
            };
            let outcome = self.execute_frame(frame, resume);
            let returned = self.last_frame.take();
            {
                if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                    g.running = false;
                }
            }
            match outcome {
                Ok(ExecOutcome::Return(v)) => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.done = true;
                        g.state = GeneratorState::Completed;
                    }
                    let result = self.iterator_result(v, true)?;
                    let resolve = Value::Object(req.cap.resolve.clone());
                    let _ = self.call_function(&resolve, &Value::Undefined, &[result]);
                }
                Ok(ExecOutcome::Yield(v)) => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.state = GeneratorState::SuspendedYield;
                        g.frame = returned;
                    }
                    let result = self.iterator_result(v, false)?;
                    let resolve = Value::Object(req.cap.resolve.clone());
                    let _ = self.call_function(&resolve, &Value::Undefined, &[result]);
                }
                Ok(ExecOutcome::Await(v)) => {
                    {
                        let mut data = gen.0.borrow_mut();
                        if let ObjectKind::AsyncGenerator(g) = &mut data.kind {
                            g.frame = returned;
                            g.pending = Some(req);
                        }
                    }
                    let awaited = self.promise_resolve_value(&v)?;
                    let then = self.get_property(&awaited, &PropertyKey::from_str("then"))?;
                    let gen_f = gen.clone();
                    let fulfill = native_fn("fulfilled", 1, move |interp, _this, args| {
                        let v = args.first().cloned().unwrap_or(Value::Undefined);
                        interp
                            .realm
                            .jobs
                            .push_back(crate::realm::Job::AsyncGenResume {
                                generator: gen_f.clone(),
                                value: v,
                                is_error: false,
                            });
                        Ok(Value::Undefined)
                    });
                    fulfill
                        .set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
                    let gen_r = gen.clone();
                    let reject = native_fn("rejected", 1, move |interp, _this, args| {
                        let v = args.first().cloned().unwrap_or(Value::Undefined);
                        interp
                            .realm
                            .jobs
                            .push_back(crate::realm::Job::AsyncGenResume {
                                generator: gen_r.clone(),
                                value: v,
                                is_error: true,
                            });
                        Ok(Value::Undefined)
                    });
                    reject
                        .set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
                    let _ = self.call_function(
                        &then,
                        &awaited,
                        &[Value::Object(fulfill), Value::Object(reject)],
                    );
                    return Ok(());
                }
                Ok(ExecOutcome::Throw(v)) => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.done = true;
                    }
                    let result = self.iterator_result(v, true)?;
                    let _ = result;
                    let reject = Value::Object(req.cap.reject.clone());
                    let _ = self.call_function(&reject, &Value::Undefined, &[Value::Undefined]);
                }
                Err(Completion::Throw(v)) => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.done = true;
                    }
                    let reject = Value::Object(req.cap.reject.clone());
                    let _ = self.call_function(&reject, &Value::Undefined, &[v]);
                }
                Err(Completion::Return(v)) => {
                    if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                        g.done = true;
                    }
                    let result = self.iterator_result(v, true)?;
                    let resolve = Value::Object(req.cap.resolve.clone());
                    let _ = self.call_function(&resolve, &Value::Undefined, &[result]);
                }
                Err(_) => {}
            }
        }
    }

    pub fn async_generator_resume(
        &mut self,
        gen: &JsObject,
        value: Value,
        is_error: bool,
    ) -> JsResult<()> {
        let (req, frame) = {
            let mut data = gen.0.borrow_mut();
            if let ObjectKind::AsyncGenerator(g) = &mut data.kind {
                (g.pending.take(), g.frame.take())
            } else {
                (None, None)
            }
        };
        let (req, frame) = match (req, frame) {
            (Some(r), Some(f)) => (r, f),
            _ => return Ok(()),
        };
        let resume = if is_error {
            Resume::Throw(value)
        } else {
            Resume::Value(value)
        };
        let outcome = self.execute_frame(frame, resume);
        let returned = self.last_frame.take();
        match outcome {
            Ok(ExecOutcome::Return(v)) => {
                if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                    g.done = true;
                }
                let result = self.iterator_result(v, true)?;
                let resolve = Value::Object(req.cap.resolve.clone());
                self.call_function(&resolve, &Value::Undefined, &[result])?;
            }
            Ok(ExecOutcome::Yield(v)) => {
                if let ObjectKind::AsyncGenerator(g) = &mut gen.0.borrow_mut().kind {
                    g.state = GeneratorState::SuspendedYield;
                    g.frame = returned;
                }
                let result = self.iterator_result(v, false)?;
                let resolve = Value::Object(req.cap.resolve.clone());
                self.call_function(&resolve, &Value::Undefined, &[result])?;
            }
            Ok(ExecOutcome::Await(v)) => {
                {
                    let mut data = gen.0.borrow_mut();
                    if let ObjectKind::AsyncGenerator(g) = &mut data.kind {
                        g.frame = returned;
                        g.pending = Some(req);
                    }
                }
                let awaited = self.promise_resolve_value(&v)?;
                let then = self.get_property(&awaited, &PropertyKey::from_str("then"))?;
                let gen_f = gen.clone();
                let fulfill = native_fn("fulfilled", 1, move |interp, _this, args| {
                    let v = args.first().cloned().unwrap_or(Value::Undefined);
                    interp
                        .realm
                        .jobs
                        .push_back(crate::realm::Job::AsyncGenResume {
                            generator: gen_f.clone(),
                            value: v,
                            is_error: false,
                        });
                    Ok(Value::Undefined)
                });
                fulfill.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
                let gen_r = gen.clone();
                let reject = native_fn("rejected", 1, move |interp, _this, args| {
                    let v = args.first().cloned().unwrap_or(Value::Undefined);
                    interp
                        .realm
                        .jobs
                        .push_back(crate::realm::Job::AsyncGenResume {
                            generator: gen_r.clone(),
                            value: v,
                            is_error: true,
                        });
                    Ok(Value::Undefined)
                });
                reject.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
                self.call_function(
                    &then,
                    &awaited,
                    &[Value::Object(fulfill), Value::Object(reject)],
                )?;
                return Ok(());
            }
            Ok(ExecOutcome::Throw(v)) => {
                let reject = Value::Object(req.cap.reject.clone());
                self.call_function(&reject, &Value::Undefined, &[v])?;
            }
            Err(Completion::Throw(v)) => {
                let reject = Value::Object(req.cap.reject.clone());
                self.call_function(&reject, &Value::Undefined, &[v])?;
            }
            Err(Completion::Return(v)) => {
                let result = self.iterator_result(v, true)?;
                let resolve = Value::Object(req.cap.resolve.clone());
                self.call_function(&resolve, &Value::Undefined, &[result])?;
            }
            Err(_) => {}
        }
        self.async_generator_process(gen)?;
        Ok(())
    }
}
