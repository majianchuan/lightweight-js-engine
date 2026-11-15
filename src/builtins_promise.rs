//! Promise implementation and microtask jobs.

use crate::builtins::native_fn;
use crate::object::*;
use crate::value::*;
use crate::vm::Interp;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

impl Interp {
    pub fn install_promise(&mut self) {
        let promise_proto = self.realm.intrinsics.promise_prototype.clone();
        let ctor = crate::builtins::native_ctor(
            "Promise",
            1,
            |interp, _this, _args| {
                interp.throw_type_error("Promise constructor cannot be invoked without 'new'")
            },
            |interp, args, new_target| {
                let executor = args.first().cloned().unwrap_or(Value::Undefined);
                if !interp.is_callable_value(&executor) {
                    return interp.throw_type_error("Promise executor is not a function");
                }
                let proto = interp.prototype_from_new_target(new_target);
                let promise = JsObject::new(ObjectData {
                    prototype: Some(proto),
                    extensible: true,
                    properties: IndexMap::new(),
                    kind: ObjectKind::Promise(PromiseData {
                        state: PromiseState::Pending,
                        result: Value::Undefined,
                        reactions: Vec::new(),
                        handled: false,
                        is_handled: false,
                    }),
                    private: IndexMap::new(),
                });
                let (resolve, reject) = interp.create_resolving_functions(&promise);
                let res = interp.call_function(
                    &executor,
                    &Value::Undefined,
                    &[Value::Object(resolve), Value::Object(reject)],
                );
                if let Err(Completion::Throw(e)) = res {
                    interp.reject_promise(&promise, e);
                }
                Ok(Value::Object(promise))
            },
        );
        ctor.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        ctor.define_fixed("prototype", Value::Object(promise_proto.clone()));
        promise_proto.put_hidden("constructor", Value::Object(ctor.clone()));
        self.realm
            .global
            .put_hidden("Promise", Value::Object(ctor.clone()));
        self.realm.intrinsics.promise_ctor = Some(ctor.clone());

        self.define_method(&promise_proto, "then", 2, |interp, this, args| {
            let promise = interp.promise_object(this)?;
            let on_f = args.first().cloned().unwrap_or(Value::Undefined);
            let on_r = args.get(1).cloned().unwrap_or(Value::Undefined);
            let cap = interp.new_promise_capability()?;
            interp.perform_promise_then(&promise, on_f, on_r, cap)
        });
        self.define_method(&promise_proto, "catch", 1, |interp, this, args| {
            let on_r = args.first().cloned().unwrap_or(Value::Undefined);
            let then = interp.get_property(this, &PropertyKey::from_str("then"))?;
            interp.call_function(&then, this, &[Value::Undefined, on_r])
        });
        self.define_method(&promise_proto, "finally", 1, |interp, this, args| {
            let on_finally = args.first().cloned().unwrap_or(Value::Undefined);
            if !interp.is_callable_value(&on_finally) {
                let then = interp.get_property(this, &PropertyKey::from_str("then"))?;
                return interp.call_function(&then, this, &[on_finally.clone(), on_finally]);
            }
            let ctor = interp.get_property(this, &PropertyKey::from_str("constructor"))?;
            let promise = interp.promise_object(this)?;
            let on_f = make_finally_handler(interp, &on_finally, &ctor, true);
            let on_r = make_finally_handler(interp, &on_finally, &ctor, false);
            let _ = promise;
            let then = interp.get_property(this, &PropertyKey::from_str("then"))?;
            interp.call_function(&then, this, &[Value::Object(on_f), Value::Object(on_r)])
        });

        self.define_method(&ctor, "resolve", 1, |interp, _this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            interp.promise_resolve_value(&v)
        });
        self.define_method(&ctor, "reject", 1, |interp, _this, args| {
            let v = args.first().cloned().unwrap_or(Value::Undefined);
            if let Some(p) = args.first().and_then(|x| x.as_object_cloned()) {
                if let ObjectKind::Promise(_) = p.borrow().kind {
                    let cap = interp.new_promise_capability()?;
                    interp.call_function(&Value::Object(cap.reject), &Value::Undefined, &[v])?;
                    return Ok(Value::Object(cap.promise));
                }
            }
            let cap = interp.new_promise_capability()?;
            interp.call_function(&Value::Object(cap.reject), &Value::Undefined, &[v])?;
            Ok(Value::Object(cap.promise))
        });
        self.define_method(&ctor, "all", 1, |interp, _this, args| {
            promise_combinator(interp, args, Combinator::All)
        });
        self.define_method(&ctor, "allSettled", 1, |interp, _this, args| {
            promise_combinator(interp, args, Combinator::AllSettled)
        });
        self.define_method(&ctor, "race", 1, |interp, _this, args| {
            promise_combinator(interp, args, Combinator::Race)
        });
        self.define_method(&ctor, "any", 1, |interp, _this, args| {
            promise_combinator(interp, args, Combinator::Any)
        });
        self.define_method(&ctor, "withResolvers", 0, |interp, _this, _args| {
            let cap = interp.new_promise_capability()?;
            let obj = JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
            interp.create_data_property(
                &obj,
                PropertyKey::from_str("promise"),
                Value::Object(cap.promise),
            )?;
            interp.create_data_property(
                &obj,
                PropertyKey::from_str("resolve"),
                Value::Object(cap.resolve),
            )?;
            interp.create_data_property(
                &obj,
                PropertyKey::from_str("reject"),
                Value::Object(cap.reject),
            )?;
            Ok(Value::Object(obj))
        });
        self.define_method(&ctor, "try", 1, |interp, _this, args| {
            let f = args.first().cloned().unwrap_or(Value::Undefined);
            let cap = interp.new_promise_capability()?;
            let rest = if args.len() > 1 {
                args[1..].to_vec()
            } else {
                Vec::new()
            };
            match interp.call_function(&f, &Value::Undefined, &rest) {
                Ok(v) => {
                    let resolve = Value::Object(cap.resolve.clone());
                    interp.call_function(&resolve, &Value::Undefined, &[v])?;
                }
                Err(Completion::Throw(e)) => {
                    let reject = Value::Object(cap.reject.clone());
                    interp.call_function(&reject, &Value::Undefined, &[e])?;
                }
                Err(c) => return Err(c),
            }
            Ok(Value::Object(cap.promise))
        });

        // Symbol.toStringTag
        let desc = Descriptor {
            value: Some(Value::String(JsString::from_str("Promise"))),
            writable: Some(false),
            enumerable: Some(false),
            configurable: Some(true),
            ..Default::default()
        };
        let _ = self.define_own_property(
            &promise_proto,
            PropertyKey::Symbol(self.realm.intrinsics.symbol_to_string_tag.clone()),
            desc,
        );
    }

    pub fn promise_object(&mut self, value: &Value) -> JsResult<JsObject> {
        if let Some(o) = value.as_object_cloned() {
            if matches!(&o.borrow().kind, ObjectKind::Promise(_)) {
                return Ok(o);
            }
        }
        self.throw_type_error("receiver is not a Promise")
    }

    pub fn new_promise_capability(&mut self) -> JsResult<PromiseCapability> {
        let proto = self.realm.intrinsics.promise_prototype.clone();
        let promise = JsObject::new(ObjectData {
            prototype: Some(proto),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Promise(PromiseData {
                state: PromiseState::Pending,
                result: Value::Undefined,
                reactions: Vec::new(),
                handled: false,
                is_handled: false,
            }),
            private: IndexMap::new(),
        });
        let (resolve, reject) = self.create_resolving_functions(&promise);
        Ok(PromiseCapability {
            promise,
            resolve,
            reject,
        })
    }

    pub fn create_resolving_functions(&mut self, promise: &JsObject) -> (JsObject, JsObject) {
        let already = Rc::new(RefCell::new(false));
        let p1 = promise.clone();
        let already1 = already.clone();
        let resolve = native_fn("resolve", 1, move |interp, _this, args| {
            if *already1.borrow() {
                return Ok(Value::Undefined);
            }
            *already1.borrow_mut() = true;
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            interp.resolve_promise_value(&p1, value);
            Ok(Value::Undefined)
        });
        resolve.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        let p2 = promise.clone();
        let already2 = already.clone();
        let reject = native_fn("reject", 1, move |interp, _this, args| {
            if *already2.borrow() {
                return Ok(Value::Undefined);
            }
            *already2.borrow_mut() = true;
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            interp.reject_promise(&p2, value);
            Ok(Value::Undefined)
        });
        reject.set_prototype_raw(Some(self.realm.intrinsics.function_prototype.clone()));
        (resolve, reject)
    }

    pub fn resolve_promise_value(&mut self, promise: &JsObject, value: Value) {
        if let Value::Object(o) = &value {
            if o.ptr_eq(promise) {
                let err = self.realm.realm_error(
                    &self.realm.intrinsics.type_error_prototype.clone(),
                    "Chaining cycle detected for promise",
                );
                self.reject_promise(promise, err);
                return;
            }
            let then = match self.get_property(&value, &PropertyKey::from_str("then")) {
                Ok(t) => t,
                Err(Completion::Throw(e)) => {
                    self.reject_promise(promise, e);
                    return;
                }
                Err(_) => return,
            };
            if self.is_callable_value(&then) {
                self.realm
                    .jobs
                    .push_back(crate::realm::Job::PromiseResolveThenable {
                        promise: promise.clone(),
                        thenable: value,
                        then,
                    });
                return;
            }
        }
        self.fulfill_promise(promise, value);
    }

    pub fn fulfill_promise(&mut self, promise: &JsObject, value: Value) {
        let reactions = {
            let mut data = promise.0.borrow_mut();
            if let ObjectKind::Promise(p) = &mut data.kind {
                if p.state != PromiseState::Pending {
                    return;
                }
                p.state = PromiseState::Fulfilled;
                p.result = value.clone();
                std::mem::take(&mut p.reactions)
            } else {
                return;
            }
        };
        self.trigger_reactions(reactions, value, ReactionKind::Fulfill);
    }

    pub fn reject_promise(&mut self, promise: &JsObject, reason: Value) {
        let reactions = {
            let mut data = promise.0.borrow_mut();
            if let ObjectKind::Promise(p) = &mut data.kind {
                if p.state != PromiseState::Pending {
                    return;
                }
                p.state = PromiseState::Rejected;
                p.result = reason.clone();
                std::mem::take(&mut p.reactions)
            } else {
                return;
            }
        };
        self.trigger_reactions(reactions, reason, ReactionKind::Reject);
    }

    fn trigger_reactions(
        &mut self,
        reactions: Vec<PromiseReaction>,
        argument: Value,
        settlement: ReactionKind,
    ) {
        for reaction in reactions {
            if reaction.kind != settlement {
                continue;
            }
            self.realm
                .jobs
                .push_back(crate::realm::Job::PromiseReaction {
                    reaction,
                    argument: argument.clone(),
                });
        }
    }

    pub fn perform_promise_then(
        &mut self,
        promise: &JsObject,
        on_fulfilled: Value,
        on_rejected: Value,
        cap: PromiseCapability,
    ) -> JsResult<Value> {
        let f = if self.is_callable_value(&on_fulfilled) {
            on_fulfilled.as_object_cloned()
        } else {
            None
        };
        let r = if self.is_callable_value(&on_rejected) {
            on_rejected.as_object_cloned()
        } else {
            None
        };
        let reaction_f = PromiseReaction {
            capability: Some(cap.clone()),
            handler: f,
            kind: ReactionKind::Fulfill,
        };
        let reaction_r = PromiseReaction {
            capability: Some(cap.clone()),
            handler: r,
            kind: ReactionKind::Reject,
        };
        let (state, result, already) = {
            let data = promise.0.borrow();
            if let ObjectKind::Promise(p) = &data.kind {
                (p.state, p.result.clone(), !p.reactions.is_empty())
            } else {
                return self.throw_type_error("receiver is not a Promise");
            }
        };
        let _ = already;
        match state {
            PromiseState::Pending => {
                let mut data = promise.0.borrow_mut();
                if let ObjectKind::Promise(p) = &mut data.kind {
                    p.reactions.push(reaction_f);
                    p.reactions.push(reaction_r);
                }
            }
            PromiseState::Fulfilled => {
                self.realm
                    .jobs
                    .push_back(crate::realm::Job::PromiseReaction {
                        reaction: reaction_f,
                        argument: result,
                    });
            }
            PromiseState::Rejected => {
                self.realm
                    .jobs
                    .push_back(crate::realm::Job::PromiseReaction {
                        reaction: reaction_r,
                        argument: result,
                    });
            }
        }
        Ok(Value::Object(cap.promise))
    }

    pub fn promise_resolve_value(&mut self, value: &Value) -> JsResult<Value> {
        if let Some(o) = value.as_object_cloned() {
            if let ObjectKind::Promise(_) = o.borrow().kind {
                return Ok(value.clone());
            }
        }
        let cap = self.new_promise_capability()?;
        let resolve = Value::Object(cap.resolve.clone());
        self.call_function(&resolve, &Value::Undefined, &[value.clone()])?;
        Ok(Value::Object(cap.promise))
    }

    pub fn run_jobs(&mut self, max_steps: usize) -> JsResult<()> {
        let mut steps = 0;
        while let Some(job) = self.realm.jobs.pop_front() {
            steps += 1;
            if steps > max_steps {
                return self.throw_range_error("microtask queue step limit exceeded");
            }
            match job {
                crate::realm::Job::PromiseReaction { reaction, argument } => {
                    let handler = reaction.handler;
                    let cap = reaction.capability;
                    if let Some(cap) = cap {
                        match handler {
                            Some(h) => match self.call_function(
                                &Value::Object(h),
                                &Value::Undefined,
                                &[argument],
                            ) {
                                Ok(v) => {
                                    let resolve = Value::Object(cap.resolve);
                                    let _ = self.call_function(&resolve, &Value::Undefined, &[v]);
                                }
                                Err(Completion::Throw(e)) => {
                                    let reject = Value::Object(cap.reject);
                                    let _ = self.call_function(&reject, &Value::Undefined, &[e]);
                                }
                                Err(c) => return Err(c),
                            },
                            None => {
                                if reaction.kind == ReactionKind::Fulfill {
                                    let resolve = Value::Object(cap.resolve);
                                    let _ = self.call_function(
                                        &resolve,
                                        &Value::Undefined,
                                        &[argument],
                                    );
                                } else {
                                    let reject = Value::Object(cap.reject);
                                    let _ =
                                        self.call_function(&reject, &Value::Undefined, &[argument]);
                                }
                            }
                        }
                    }
                }
                crate::realm::Job::PromiseResolveThenable {
                    promise,
                    thenable,
                    then,
                } => {
                    let (resolve, reject) = self.create_resolving_functions(&promise);
                    let res = self.call_function(
                        &then,
                        &thenable,
                        &[Value::Object(resolve), Value::Object(reject)],
                    );
                    if let Err(Completion::Throw(e)) = res {
                        self.reject_promise(&promise, e);
                    }
                }
                crate::realm::Job::AsyncGenResume {
                    generator,
                    value,
                    is_error,
                } => {
                    let _ = self.async_generator_resume(&generator, value, is_error);
                }
                crate::realm::Job::ResumeAsync {
                    state,
                    value,
                    is_error,
                } => {
                    let _ = self.resume_async_job(state, value, is_error);
                }
            }
        }
        Ok(())
    }
}

fn make_finally_handler(
    interp: &mut Interp,
    on_finally: &Value,
    ctor: &Value,
    fulfilled: bool,
) -> JsObject {
    let on_finally = on_finally.clone();
    let ctor = ctor.clone();
    let f = native_fn(
        if fulfilled { "onFinally" } else { "onFinally" },
        1,
        move |interp, _this, args| {
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let r = interp.call_function(&on_finally, &Value::Undefined, &[])?;
            let promise_ctor = if ctor.is_object() {
                ctor.clone()
            } else {
                Value::Object(interp.realm.intrinsics.promise_ctor.clone().unwrap())
            };
            let then = interp.get_property(&promise_ctor, &PropertyKey::from_str("then"))?;
            let resolved = interp.call_function(&then, &promise_ctor, &[r])?;
            let _ = resolved;
            if fulfilled {
                Ok(value)
            } else {
                Err(Completion::Throw(value))
            }
        },
    );
    f.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
    f
}

#[derive(Clone, Copy)]
enum Combinator {
    All,
    AllSettled,
    Race,
    Any,
}

fn promise_combinator(interp: &mut Interp, args: &[Value], kind: Combinator) -> JsResult<Value> {
    let iterable = args.first().cloned().unwrap_or(Value::Undefined);
    let cap = interp.new_promise_capability()?;
    let values = match interp.iterable_to_list(&iterable) {
        Ok(v) => v,
        Err(c) => {
            let reject = Value::Object(cap.reject.clone());
            let _ = interp.call_function(&reject, &Value::Undefined, &[error_value(&c)]);
            return Ok(Value::Object(cap.promise));
        }
    };
    let result_values: Rc<RefCell<Vec<Value>>> =
        Rc::new(RefCell::new(vec![Value::Undefined; values.len()]));
    let remaining = Rc::new(RefCell::new(values.len()));
    let errors: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
    let outer_promise = cap.promise.clone();
    let resolve = cap.resolve.clone();
    let reject = cap.reject.clone();
    for (i, v) in values.into_iter().enumerate() {
        let resolved = match interp.promise_resolve_value(&v) {
            Ok(r) => r,
            Err(c) => {
                let _ = interp.call_function(
                    &Value::Object(reject.clone()),
                    &Value::Undefined,
                    &[error_value(&c)],
                );
                return Ok(Value::Object(outer_promise));
            }
        };
        let then = interp.get_property(&resolved, &PropertyKey::from_str("then"))?;
        let values_c = result_values.clone();
        let remaining_c = remaining.clone();
        let errors_c = errors.clone();
        let resolve_c = resolve.clone();
        let reject_c = reject.clone();
        let outer_c = outer_promise.clone();
        let on_f = native_fn("onFulfilled", 1, move |interp, _this, args| {
            let val = args.first().cloned().unwrap_or(Value::Undefined);
            match kind {
                Combinator::All => {
                    values_c.borrow_mut()[i] = val;
                }
                Combinator::AllSettled => {
                    let obj =
                        JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
                    obj.put("status", Value::String(JsString::from_str("fulfilled")));
                    obj.put("value", val);
                    values_c.borrow_mut()[i] = Value::Object(obj);
                }
                Combinator::Race => {
                    let res = interp.call_function(
                        &Value::Object(resolve_c.clone()),
                        &Value::Undefined,
                        &[val],
                    );
                    let _ = res;
                    return Ok(Value::Undefined);
                }
                Combinator::Any => {
                    values_c.borrow_mut()[i] = val;
                }
            }
            settle_combinator(
                interp,
                &remaining_c,
                &values_c,
                &errors_c,
                &resolve_c,
                &reject_c,
                &outer_c,
                kind,
                i,
            )
        });
        on_f.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
        let values_c2 = result_values.clone();
        let remaining_c2 = remaining.clone();
        let errors_c2 = errors.clone();
        let resolve_c2 = resolve.clone();
        let reject_c2 = reject.clone();
        let outer_c2 = outer_promise.clone();
        let on_r = native_fn("onRejected", 1, move |interp, _this, args| {
            let err = args.first().cloned().unwrap_or(Value::Undefined);
            match kind {
                Combinator::All | Combinator::Race => {
                    let res = interp.call_function(
                        &Value::Object(reject_c2.clone()),
                        &Value::Undefined,
                        &[err],
                    );
                    let _ = res;
                }
                Combinator::AllSettled => {
                    let obj =
                        JsObject::ordinary(Some(interp.realm.intrinsics.object_prototype.clone()));
                    obj.put("status", Value::String(JsString::from_str("rejected")));
                    obj.put("reason", err);
                    values_c2.borrow_mut()[i] = Value::Object(obj);
                    return settle_combinator(
                        interp,
                        &remaining_c2,
                        &values_c2,
                        &errors_c2,
                        &resolve_c2,
                        &reject_c2,
                        &outer_c2,
                        kind,
                        i,
                    );
                }
                Combinator::Any => {
                    errors_c2.borrow_mut().push(err);
                    return settle_combinator(
                        interp,
                        &remaining_c2,
                        &values_c2,
                        &errors_c2,
                        &resolve_c2,
                        &reject_c2,
                        &outer_c2,
                        kind,
                        i,
                    );
                }
            }
            Ok(Value::Undefined)
        });
        on_r.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
        let then = then;
        let _ = interp.call_function(
            &then,
            &resolved,
            &[Value::Object(on_f), Value::Object(on_r)],
        );
    }
    if values_is_empty(&result_values) {
        let arr = interp.create_array_from_list(Vec::new())?;
        let _ = interp.call_function(
            &Value::Object(resolve),
            &Value::Undefined,
            &[Value::Object(arr)],
        );
    }
    Ok(Value::Object(outer_promise))
}

fn values_is_empty(v: &Rc<RefCell<Vec<Value>>>) -> bool {
    v.borrow().is_empty()
}

#[allow(clippy::too_many_arguments)]
fn settle_combinator(
    interp: &mut Interp,
    remaining: &Rc<RefCell<usize>>,
    values: &Rc<RefCell<Vec<Value>>>,
    errors: &Rc<RefCell<Vec<Value>>>,
    resolve: &JsObject,
    reject: &JsObject,
    _outer: &JsObject,
    kind: Combinator,
    _index: usize,
) -> JsResult<Value> {
    let mut rem = remaining.borrow_mut();
    *rem = rem.saturating_sub(1);
    if *rem == 0 {
        match kind {
            Combinator::All | Combinator::Race => {
                let arr = interp.create_array_from_list(values.borrow().clone())?;
                interp.call_function(
                    &Value::Object(resolve.clone()),
                    &Value::Undefined,
                    &[Value::Object(arr)],
                )?;
            }
            Combinator::AllSettled => {
                let arr = interp.create_array_from_list(values.borrow().clone())?;
                interp.call_function(
                    &Value::Object(resolve.clone()),
                    &Value::Undefined,
                    &[Value::Object(arr)],
                )?;
            }
            Combinator::Any => {
                if errors.borrow().is_empty() {
                    let arr = interp.create_array_from_list(values.borrow().clone())?;
                    interp.call_function(
                        &Value::Object(resolve.clone()),
                        &Value::Undefined,
                        &[Value::Object(arr)],
                    )?;
                } else {
                    let err = JsObject::ordinary(Some(
                        interp.realm.intrinsics.aggregate_error_prototype.clone(),
                    ));
                    err.put("name", Value::String(JsString::from_str("AggregateError")));
                    err.put(
                        "message",
                        Value::String(JsString::from_str("All promises were rejected")),
                    );
                    let errs = interp.create_array_from_list(errors.borrow().clone())?;
                    err.put("errors", Value::Object(errs));
                    interp.call_function(
                        &Value::Object(reject.clone()),
                        &Value::Undefined,
                        &[Value::Object(err)],
                    )?;
                }
            }
        }
    }
    Ok(Value::Undefined)
}

fn error_value(c: &Completion) -> Value {
    match c {
        Completion::Throw(v) => v.clone(),
        _ => Value::Undefined,
    }
}
