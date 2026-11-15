//! Async function support.

use crate::builtins::native_fn;
use crate::env::EnvRef;
use crate::object::*;
use crate::value::*;
use crate::vm::{AsyncState, ExecOutcome, Frame, Interp, Resume};
use std::cell::RefCell;
use std::rc::Rc;

impl Interp {
    pub fn install_async(&mut self) {
        // AsyncFunction.prototype exists mostly for `Object.getPrototypeOf`.
        let proto = self
            .realm
            .intrinsics
            .async_generator_function_prototype
            .clone();
        let _ = proto;
    }

    pub fn create_async_function(
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
        let cap = self.new_promise_capability()?;
        let state = Rc::new(AsyncState {
            frame: RefCell::new(Some(frame)),
            resolve: cap.resolve.clone(),
            reject: cap.reject.clone(),
        });
        self.run_async(state, Resume::Normal)?;
        Ok(Value::Object(cap.promise))
    }

    pub fn run_async(&mut self, state: Rc<AsyncState>, resume: Resume) -> JsResult<()> {
        let frame = state.frame.borrow_mut().take();
        let frame = match frame {
            Some(f) => f,
            None => return Ok(()),
        };
        let outcome = self.execute_frame(frame, resume);
        let returned = self.last_frame.take();
        match outcome {
            Ok(ExecOutcome::Return(v)) => {
                let resolve = Value::Object(state.resolve.clone());
                let _ = self.call_function(&resolve, &Value::Undefined, &[v]);
            }
            Ok(ExecOutcome::Throw(v)) => {
                let reject = Value::Object(state.reject.clone());
                let _ = self.call_function(&reject, &Value::Undefined, &[v]);
            }
            Ok(ExecOutcome::Await(v)) => {
                *state.frame.borrow_mut() = returned;
                let awaited = self.promise_resolve_value(&v)?;
                let then = self.get_property(&awaited, &PropertyKey::from_str("then"))?;
                let state_f = state.clone();
                let fulfill = native_fn("fulfilled", 1, move |interp, _this, args| {
                    let v = args.first().cloned().unwrap_or(Value::Undefined);
                    interp.realm.jobs.push_back(crate::realm::Job::ResumeAsync {
                        state: state_f.clone(),
                        value: v,
                        is_error: false,
                    });
                    Ok(Value::Undefined)
                });
                fulfill.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
                let state_r = state.clone();
                let reject = native_fn("rejected", 1, move |interp, _this, args| {
                    let v = args.first().cloned().unwrap_or(Value::Undefined);
                    interp.realm.jobs.push_back(crate::realm::Job::ResumeAsync {
                        state: state_r.clone(),
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
            }
            Ok(ExecOutcome::Yield(_)) => {
                let reject = Value::Object(state.reject.clone());
                let _ = reject;
            }
            Err(Completion::Return(v)) => {
                let resolve = Value::Object(state.resolve.clone());
                let _ = self.call_function(&resolve, &Value::Undefined, &[v]);
            }
            Err(Completion::Throw(v)) => {
                let reject = Value::Object(state.reject.clone());
                let _ = self.call_function(&reject, &Value::Undefined, &[v]);
            }
            Err(_) => {}
        }
        Ok(())
    }

    pub fn resume_async_job(
        &mut self,
        state: Rc<AsyncState>,
        value: Value,
        is_error: bool,
    ) -> JsResult<()> {
        let resume = if is_error {
            Resume::Throw(value)
        } else {
            Resume::Value(value)
        };
        self.run_async(state, resume)
    }
}
