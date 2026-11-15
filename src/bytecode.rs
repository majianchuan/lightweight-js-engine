//! Bytecode representation for the engine.

use crate::value::{JsString, Value};
use num_bigint::BigInt;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Constant {
    Number(f64),
    String(JsString),
    BigInt(Rc<BigInt>),
    RegExp { source: JsString, flags: JsString },
    Symbol(crate::value::SymbolValue),
    Template(TemplateSite),
    Class(Rc<ClassInfo>),
}

#[derive(Clone, Debug)]
pub struct TemplateSite {
    pub cooked: Vec<Option<JsString>>,
    pub raw: Vec<JsString>,
}

#[derive(Clone, Debug)]
pub struct ClassInfo {
    pub name: JsString,
    pub constructor: Option<Arc<FunctionProto>>,
    pub elements: Vec<BytecodeClassElement>,
    pub private_names: Vec<JsString>,
}

#[derive(Clone, Debug)]
pub struct BytecodeClassElement {
    pub key: ClassKey,
    pub kind: ClassElementKind,
    pub is_static: bool,
    pub function: Option<Arc<FunctionProto>>,
}

#[derive(Clone, Debug)]
pub enum ClassKey {
    Named(JsString),
    Computed(JsString),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassElementKind {
    Method,
    Getter,
    Setter,
    Field,
    StaticBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    In,
    InstanceOf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    BitNot,
    TypeOf,
    Void,
    Delete,
    Increment,
    Decrement,
}

#[derive(Clone, Debug)]
pub enum Op {
    Nop,
    PushUndefined,
    PushNull,
    PushTrue,
    PushFalse,
    PushConst(u32),

    Pop,
    Dup,
    Swap,
    Rot3,

    LoadVar {
        depth: u16,
        slot: u16,
    },
    StoreVar {
        depth: u16,
        slot: u16,
    },
    InitVar {
        depth: u16,
        slot: u16,
    },
    LoadGlobal(u32),
    StoreGlobal {
        name: u32,
        strict: bool,
    },
    TypeOfGlobal(u32),
    DeleteGlobal(u32),
    DeclareGlobalVar(u32),
    StoreGlobalFunction(u32),
    LoadThis,
    LoadNewTarget,
    LoadArg(u32),
    LoadArgsCount,
    LoadArgsObject,
    CreateRestArray(u32),
    CreateArguments,

    CreateEnv(u32),
    CreateWithEnv,
    LoadName {
        name: u32,
        depth: u16,
        slot: u16,
        has_local: bool,
    },
    StoreName {
        name: u32,
        depth: u16,
        slot: u16,
        has_local: bool,
        strict: bool,
    },
    TypeOfName {
        name: u32,
        depth: u16,
        slot: u16,
        has_local: bool,
    },
    PopEnv,

    CreateClosure(u32),
    CreateClosureWithHome(u32),
    NewObject,
    NewArray,
    ArrayPush,
    ArrayPushSpread,
    SetArrayLength(u32),
    DefineDataProp {
        key: u32,
        computed: bool,
    },
    DefineAccessorProp {
        key: u32,
        computed: bool,
        getter: bool,
    },
    SetProtoLiteral,
    SetPrototypeOf,
    DefineClassPrototype,
    DefineClassConstructor,
    DefineClassMethod {
        kind: u8,
    },
    AddClassField {
        is_static: bool,
    },
    PushArgsArray,
    CopyDataProperties,
    GetProp(u32),
    SetProp(u32),
    GetElem,
    SetElem,
    DeleteProp(u32),
    DeleteElem,
    DefineOwnDataProp(u32),
    GetSuperProp(u32),
    SetSuperProp(u32),
    GetSuperElem,
    SetSuperElem,
    PrivateGet(u32),
    PrivateSet(u32),
    PrivateIn(u32),

    Binary(BinOp),
    Unary(UnaryOp),
    ToPropertyKey,
    ToNumeric,
    ToString,
    CopyDataPropertiesExcluding(u32),

    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    JumpIfFalsePop(u32),
    JumpIfTruePop(u32),
    JumpIfNullishPop(u32),
    JumpIfNotNullishPop(u32),
    JumpIfNotUndefined(u32),

    Call(u32),
    CallSpread,
    New(u32),
    NewSpread,
    SuperCall(u32),
    SuperCallSpread,

    Return,
    ReturnUndefined,
    Throw,

    LoadTemp(u32),
    StoreTemp(u32),
    StoreTempKeep(u32),

    IterInit(u32),
    AsyncIterInit(u32),
    AsyncIterNext(u32),
    IterNext {
        temp: u32,
        done: u32,
    },
    IterNextDestructure {
        temp: u32,
        done: u32,
    },
    IterRest {
        temp: u32,
    },
    IterClose {
        temp: u32,
    },
    ForInInit(u32),
    ForInNext {
        temp: u32,
        done: u32,
    },

    CreateRegExp(u32),
    TemplateObject(u32),

    SetNormalCompletion(u32),
    EndFinally,

    Yield,
    Await,
    AsyncReturn,
    ThrowConstAssignment,
}

#[derive(Clone, Debug, Default)]
pub struct Handler {
    pub start: u32,
    pub end: u32,
    pub catch_target: Option<u32>,
    pub finally_target: Option<u32>,
    pub env_depth: u32,
    pub stack_height: u32,
    pub outer: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoKind {
    Script,
    Function,
    Arrow,
    Method,
    Getter,
    Setter,
    ClassConstructor,
    DerivedConstructor,
    Generator,
    Async,
    AsyncGenerator,
    StaticBlock,
}

impl ProtoKind {
    pub fn is_generator(self) -> bool {
        matches!(self, ProtoKind::Generator | ProtoKind::AsyncGenerator)
    }
    pub fn is_async(self) -> bool {
        matches!(self, ProtoKind::Async | ProtoKind::AsyncGenerator)
    }
    pub fn is_arrow(self) -> bool {
        matches!(self, ProtoKind::Arrow)
    }
    pub fn is_method(self) -> bool {
        matches!(
            self,
            ProtoKind::Method
                | ProtoKind::Getter
                | ProtoKind::Setter
                | ProtoKind::ClassConstructor
                | ProtoKind::DerivedConstructor
                | ProtoKind::StaticBlock
        )
    }
    pub fn is_constructor(self) -> bool {
        matches!(
            self,
            ProtoKind::Function | ProtoKind::ClassConstructor | ProtoKind::DerivedConstructor
        )
    }
}

#[derive(Clone, Debug)]
pub struct FunctionProto {
    pub name: JsString,
    pub kind: ProtoKind,
    pub body: Vec<Op>,
    pub constants: Vec<Constant>,
    pub functions: Vec<Arc<FunctionProto>>,
    pub handlers: Vec<Handler>,
    pub env_slots: u32,
    pub temp_slots: u32,
    pub length: u32,
    pub is_strict: bool,
    pub is_arrow: bool,
    pub source_text: Option<JsString>,
    pub line: u32,
    pub column: u32,
}

impl FunctionProto {
    pub fn new(name: JsString, kind: ProtoKind) -> Self {
        FunctionProto {
            name,
            kind,
            body: Vec::new(),
            constants: Vec::new(),
            functions: Vec::new(),
            handlers: Vec::new(),
            env_slots: 0,
            temp_slots: 0,
            length: 0,
            is_strict: false,
            is_arrow: false,
            source_text: None,
            line: 0,
            column: 0,
        }
    }
}

/// A value that can be stored on the VM operand stack.
pub type Slot = Value;
