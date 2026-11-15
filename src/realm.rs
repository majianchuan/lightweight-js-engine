//! Realm: the set of intrinsic objects and global state.

use crate::object::{JsObject, ObjectData, ObjectKind, PromiseData, PromiseState};
use crate::value::{JsString, PropertyKey, SymbolValue, Value};
use indexmap::IndexMap;
use std::collections::{HashMap, VecDeque};

pub struct Intrinsics {
    pub object_prototype: JsObject,
    pub function_prototype: JsObject,
    pub array_prototype: JsObject,
    pub string_prototype: JsObject,
    pub number_prototype: JsObject,
    pub boolean_prototype: JsObject,
    pub symbol_prototype: JsObject,
    pub bigint_prototype: JsObject,
    pub error_prototype: JsObject,
    pub type_error_prototype: JsObject,
    pub range_error_prototype: JsObject,
    pub reference_error_prototype: JsObject,
    pub syntax_error_prototype: JsObject,
    pub eval_error_prototype: JsObject,
    pub uri_error_prototype: JsObject,
    pub aggregate_error_prototype: JsObject,
    pub suppressed_error_prototype: JsObject,
    pub iterator_prototype: JsObject,
    pub array_iterator_prototype: JsObject,
    pub string_iterator_prototype: JsObject,
    pub map_iterator_prototype: JsObject,
    pub set_iterator_prototype: JsObject,
    pub generator_prototype: JsObject,
    pub generator_function_prototype: JsObject,
    pub async_iterator_prototype: JsObject,
    pub async_generator_prototype: JsObject,
    pub async_generator_function_prototype: JsObject,
    pub async_from_sync_iterator_prototype: JsObject,
    pub regexp_prototype: JsObject,
    pub date_prototype: JsObject,
    pub promise_prototype: JsObject,
    pub map_prototype: JsObject,
    pub set_prototype: JsObject,
    pub weak_map_prototype: JsObject,
    pub weak_set_prototype: JsObject,
    pub weak_ref_prototype: JsObject,
    pub finalization_registry_prototype: JsObject,
    pub array_buffer_prototype: JsObject,
    pub shared_array_buffer_prototype: JsObject,
    pub data_view_prototype: JsObject,
    pub typed_array_prototype: JsObject,
    pub typed_array_prototypes: Vec<(crate::object::TypedArrayKind, JsObject)>,
    pub module_namespace_prototype: JsObject,
    pub shadow_realm_prototype: JsObject,

    pub object_ctor: Option<JsObject>,
    pub function_ctor: Option<JsObject>,
    pub array_ctor: Option<JsObject>,
    pub string_ctor: Option<JsObject>,
    pub number_ctor: Option<JsObject>,
    pub boolean_ctor: Option<JsObject>,
    pub symbol_ctor: Option<JsObject>,
    pub bigint_ctor: Option<JsObject>,
    pub error_ctor: Option<JsObject>,
    pub type_error_ctor: Option<JsObject>,
    pub range_error_ctor: Option<JsObject>,
    pub reference_error_ctor: Option<JsObject>,
    pub syntax_error_ctor: Option<JsObject>,
    pub eval_error_ctor: Option<JsObject>,
    pub uri_error_ctor: Option<JsObject>,
    pub aggregate_error_ctor: Option<JsObject>,
    pub suppressed_error_ctor: Option<JsObject>,
    pub promise_ctor: Option<JsObject>,
    pub map_ctor: Option<JsObject>,
    pub set_ctor: Option<JsObject>,
    pub weak_map_ctor: Option<JsObject>,
    pub weak_set_ctor: Option<JsObject>,
    pub weak_ref_ctor: Option<JsObject>,
    pub finalization_registry_ctor: Option<JsObject>,
    pub regexp_ctor: Option<JsObject>,
    pub date_ctor: Option<JsObject>,
    pub array_buffer_ctor: Option<JsObject>,
    pub shared_array_buffer_ctor: Option<JsObject>,
    pub data_view_ctor: Option<JsObject>,
    pub typed_array_ctors: Vec<(crate::object::TypedArrayKind, JsObject)>,
    pub typed_array_ctor: Option<JsObject>,
    pub generator_function_ctor: Option<JsObject>,
    pub async_function_ctor: Option<JsObject>,
    pub async_generator_function_ctor: Option<JsObject>,

    pub symbol_iterator: SymbolValue,
    pub symbol_async_iterator: SymbolValue,
    pub symbol_has_instance: SymbolValue,
    pub symbol_is_concat_spreadable: SymbolValue,
    pub symbol_match: SymbolValue,
    pub symbol_match_all: SymbolValue,
    pub symbol_replace: SymbolValue,
    pub symbol_search: SymbolValue,
    pub symbol_species: SymbolValue,
    pub symbol_split: SymbolValue,
    pub symbol_to_primitive: SymbolValue,
    pub symbol_to_string_tag: SymbolValue,
    pub symbol_unscopables: SymbolValue,
    pub symbol_registry: HashMap<JsString, SymbolValue>,
    pub symbol_counter: u64,
}

impl Intrinsics {
    pub fn well_known(&self, id: u64) -> Option<&SymbolValue> {
        let candidates = [
            &self.symbol_iterator,
            &self.symbol_async_iterator,
            &self.symbol_has_instance,
            &self.symbol_is_concat_spreadable,
            &self.symbol_match,
            &self.symbol_match_all,
            &self.symbol_replace,
            &self.symbol_search,
            &self.symbol_species,
            &self.symbol_split,
            &self.symbol_to_primitive,
            &self.symbol_to_string_tag,
            &self.symbol_unscopables,
        ];
        candidates.iter().find(|s| s.id == id).copied()
    }
}

pub struct Realm {
    pub global: JsObject,
    pub intrinsics: Intrinsics,
    pub jobs: VecDeque<Job>,
    pub template_cache: Vec<Option<JsObject>>,
    pub module_cache: HashMap<String, JsObject>,
    pub regexp_cache: HashMap<JsString, Option<std::rc::Rc<regress::Regex>>>,
}

pub enum Job {
    PromiseReaction {
        reaction: crate::object::PromiseReaction,
        argument: Value,
    },
    PromiseResolveThenable {
        promise: JsObject,
        thenable: Value,
        then: Value,
    },
    ResumeAsync {
        state: std::rc::Rc<crate::vm::AsyncState>,
        value: Value,
        is_error: bool,
    },
    AsyncGenResume {
        generator: JsObject,
        value: Value,
        is_error: bool,
    },
}

pub type AsyncState = std::rc::Rc<crate::vm::AsyncState>;

impl Realm {
    pub fn new() -> Self {
        let object_prototype = JsObject::ordinary(None);
        let function_prototype = JsObject::native_function(
            Some(object_prototype.clone()),
            "",
            0,
            std::rc::Rc::new(|_interp, _this, _args| Ok(Value::Undefined)),
            None,
        );
        let array_prototype = JsObject::array(Some(object_prototype.clone()));
        let string_prototype = JsObject::new(ObjectData {
            prototype: Some(object_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::StringObject(JsString::empty()),
            private: IndexMap::new(),
        });
        let number_prototype = JsObject::new(ObjectData {
            prototype: Some(object_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::NumberObject(0.0),
            private: IndexMap::new(),
        });
        let boolean_prototype = JsObject::new(ObjectData {
            prototype: Some(object_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::BooleanObject(false),
            private: IndexMap::new(),
        });
        let symbol_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let bigint_prototype = JsObject::ordinary(Some(object_prototype.clone()));

        let make_error_proto = |name: &str, proto: &JsObject| {
            let o = JsObject::ordinary(Some(proto.clone()));
            o.define_fixed("name", Value::String(JsString::from_str(name)));
            o.define_fixed("message", Value::String(JsString::empty()));
            o
        };
        let error_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        error_prototype.define_fixed("name", Value::String(JsString::from_str("Error")));
        error_prototype.define_fixed("message", Value::String(JsString::empty()));
        let type_error_prototype = make_error_proto("TypeError", &error_prototype);
        let range_error_prototype = make_error_proto("RangeError", &error_prototype);
        let reference_error_prototype = make_error_proto("ReferenceError", &error_prototype);
        let syntax_error_prototype = make_error_proto("SyntaxError", &error_prototype);
        let eval_error_prototype = make_error_proto("EvalError", &error_prototype);
        let uri_error_prototype = make_error_proto("URIError", &error_prototype);
        let aggregate_error_prototype = make_error_proto("AggregateError", &error_prototype);
        let suppressed_error_prototype = make_error_proto("SuppressedError", &error_prototype);

        let iterator_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let array_iterator_prototype = JsObject::ordinary(Some(iterator_prototype.clone()));
        let string_iterator_prototype = JsObject::ordinary(Some(iterator_prototype.clone()));
        let map_iterator_prototype = JsObject::ordinary(Some(iterator_prototype.clone()));
        let set_iterator_prototype = JsObject::ordinary(Some(iterator_prototype.clone()));
        let generator_prototype = JsObject::ordinary(Some(iterator_prototype.clone()));
        let generator_function_prototype = JsObject::ordinary(Some(function_prototype.clone()));
        let async_iterator_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let async_generator_prototype = JsObject::ordinary(Some(async_iterator_prototype.clone()));
        let async_generator_function_prototype =
            JsObject::ordinary(Some(function_prototype.clone()));
        let async_from_sync_iterator_prototype =
            JsObject::ordinary(Some(async_iterator_prototype.clone()));
        let regexp_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let date_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let promise_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let map_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let set_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let weak_map_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let weak_set_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let weak_ref_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let finalization_registry_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let array_buffer_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let shared_array_buffer_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let data_view_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let typed_array_prototype = JsObject::ordinary(Some(object_prototype.clone()));
        let module_namespace_prototype = JsObject::ordinary(None);
        let shadow_realm_prototype = JsObject::ordinary(Some(object_prototype.clone()));

        let symbol_counter = 1000u64;
        let mk_symbol = |name: &str, id: u64| SymbolValue {
            id,
            description: Some(JsString::from_str(name)),
        };
        let symbol_iterator = mk_symbol("Symbol.iterator", 4);
        let symbol_async_iterator = mk_symbol("Symbol.asyncIterator", 1);
        let symbol_has_instance = mk_symbol("Symbol.hasInstance", 2);
        let symbol_is_concat_spreadable = mk_symbol("Symbol.isConcatSpreadable", 3);
        let symbol_match = mk_symbol("Symbol.match", 5);
        let symbol_match_all = mk_symbol("Symbol.matchAll", 6);
        let symbol_replace = mk_symbol("Symbol.replace", 7);
        let symbol_search = mk_symbol("Symbol.search", 8);
        let symbol_species = mk_symbol("Symbol.species", 9);
        let symbol_split = mk_symbol("Symbol.split", 10);
        let symbol_to_primitive = mk_symbol("Symbol.toPrimitive", 11);
        let symbol_to_string_tag = mk_symbol("Symbol.toStringTag", 12);
        let symbol_unscopables = mk_symbol("Symbol.unscopables", 13);

        let intrinsics = Intrinsics {
            object_prototype: object_prototype.clone(),
            function_prototype: function_prototype.clone(),
            array_prototype,
            string_prototype,
            number_prototype,
            boolean_prototype,
            symbol_prototype,
            bigint_prototype,
            error_prototype,
            type_error_prototype,
            range_error_prototype,
            reference_error_prototype,
            syntax_error_prototype,
            eval_error_prototype,
            uri_error_prototype,
            aggregate_error_prototype,
            suppressed_error_prototype,
            iterator_prototype,
            array_iterator_prototype,
            string_iterator_prototype,
            map_iterator_prototype,
            set_iterator_prototype,
            generator_prototype,
            generator_function_prototype,
            async_iterator_prototype,
            async_generator_prototype,
            async_generator_function_prototype,
            async_from_sync_iterator_prototype,
            regexp_prototype,
            date_prototype,
            promise_prototype,
            map_prototype,
            set_prototype,
            weak_map_prototype,
            weak_set_prototype,
            weak_ref_prototype,
            finalization_registry_prototype,
            array_buffer_prototype,
            shared_array_buffer_prototype,
            data_view_prototype,
            typed_array_prototype,
            typed_array_prototypes: Vec::new(),
            module_namespace_prototype,
            shadow_realm_prototype,
            object_ctor: None,
            function_ctor: None,
            array_ctor: None,
            string_ctor: None,
            number_ctor: None,
            boolean_ctor: None,
            symbol_ctor: None,
            bigint_ctor: None,
            error_ctor: None,
            type_error_ctor: None,
            range_error_ctor: None,
            reference_error_ctor: None,
            syntax_error_ctor: None,
            eval_error_ctor: None,
            uri_error_ctor: None,
            aggregate_error_ctor: None,
            suppressed_error_ctor: None,
            promise_ctor: None,
            map_ctor: None,
            set_ctor: None,
            weak_map_ctor: None,
            weak_set_ctor: None,
            weak_ref_ctor: None,
            finalization_registry_ctor: None,
            regexp_ctor: None,
            date_ctor: None,
            array_buffer_ctor: None,
            shared_array_buffer_ctor: None,
            data_view_ctor: None,
            typed_array_ctors: Vec::new(),
            typed_array_ctor: None,
            generator_function_ctor: None,
            async_function_ctor: None,
            async_generator_function_ctor: None,
            symbol_iterator,
            symbol_async_iterator,
            symbol_has_instance,
            symbol_is_concat_spreadable,
            symbol_match,
            symbol_match_all,
            symbol_replace,
            symbol_search,
            symbol_species,
            symbol_split,
            symbol_to_primitive,
            symbol_to_string_tag,
            symbol_unscopables,
            symbol_registry: HashMap::new(),
            symbol_counter,
        };

        let global = JsObject::ordinary(Some(object_prototype.clone()));
        global.put_hidden(
            PropertyKey::from_str("globalThis"),
            Value::Object(global.clone()),
        );
        global.define_fixed("undefined", Value::Undefined);
        global.define_fixed("NaN", Value::Number(f64::NAN));
        global.define_fixed("Infinity", Value::Number(f64::INFINITY));

        Realm {
            global,
            intrinsics,
            jobs: VecDeque::new(),
            template_cache: Vec::new(),
            module_cache: HashMap::new(),
            regexp_cache: HashMap::new(),
        }
    }

    pub fn global_this(&self) -> Value {
        Value::Object(self.global.clone())
    }

    pub fn new_symbol(&mut self, description: Option<JsString>) -> SymbolValue {
        let id = self.intrinsics.symbol_counter;
        self.intrinsics.symbol_counter += 1;
        SymbolValue { id, description }
    }

    pub fn symbol_for(&mut self, key: JsString) -> SymbolValue {
        if let Some(s) = self.intrinsics.symbol_registry.get(&key) {
            return s.clone();
        }
        let s = self.new_symbol(Some(key.clone()));
        self.intrinsics.symbol_registry.insert(key, s.clone());
        s
    }

    pub fn new_promise(&self, state: PromiseState, result: Value) -> JsObject {
        JsObject::new(ObjectData {
            prototype: Some(self.intrinsics.promise_prototype.clone()),
            extensible: true,
            properties: IndexMap::new(),
            kind: ObjectKind::Promise(PromiseData {
                state,
                result,
                reactions: Vec::new(),
                handled: false,
                is_handled: false,
            }),
            private: IndexMap::new(),
        })
    }

    pub fn realm_error(&self, proto: &JsObject, message: &str) -> Value {
        let obj = JsObject::ordinary(Some(proto.clone()));
        obj.put_hidden("message", Value::String(JsString::from_str(message)));
        Value::Object(obj)
    }
}
