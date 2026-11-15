//! AST -> bytecode compiler.

use crate::bytecode::*;
use crate::value::{JsString, SymbolValue};
use num_bigint::BigInt;
use oxc_ast::ast::*;
use oxc_span::GetSpan;
use oxc_syntax::operator::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0;
    while let Some(pos) = text[start..].find(word) {
        let at = start + pos;
        let before = at == 0
            || (!(bytes[at - 1] as char).is_alphanumeric()
                && bytes[at - 1] != 95
                && bytes[at - 1] != 36);
        let after = at + word.len() >= bytes.len()
            || (!(bytes[at + word.len()] as char).is_alphanumeric()
                && bytes[at + word.len()] != 95
                && bytes[at + word.len()] != 36);
        if before && after {
            return true;
        }
        start = at + word.len();
    }
    false
}

pub struct CompileError {
    pub message: String,
}

impl CompileError {
    fn new(msg: impl Into<String>) -> Self {
        CompileError {
            message: msg.into(),
        }
    }
}

type CResult<T> = Result<T, CompileError>;

pub fn compile_script(source: &str, program: &Program) -> CResult<Arc<FunctionProto>> {
    compile_script_mode(source, program, false)
}

pub fn compile_eval(source: &str, program: &Program) -> CResult<Arc<FunctionProto>> {
    compile_script_mode(source, program, true)
}

fn compile_script_mode(
    source: &str,
    program: &Program,
    eval_mode: bool,
) -> CResult<Arc<FunctionProto>> {
    let strict = program
        .directives
        .iter()
        .any(|d| d.directive.as_str() == "use strict");
    let mut c = Compiler::new(source, strict, ProtoKind::Script);
    c.eval_mode = eval_mode;
    c.compile_script_program(program)?;
    Ok(Arc::new(c.finish()))
}

pub fn compile_function_source(source: &str, program: &Program) -> CResult<Arc<FunctionProto>> {
    compile_script(source, program)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Function,
    Script,
    Block,
    Catch,
}

#[derive(Clone)]
struct Binding {
    slot: u16,
    is_const: bool,
}

struct Scope {
    parent: Option<usize>,
    depth: u32,
    kind: ScopeKind,
    slots: Vec<u16>,
    next_slot: u16,
    bindings: HashMap<JsString, Binding>,
}

struct LoopCtx {
    label: Option<JsString>,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    scope_depth: u32,
    is_loop: bool,
    continue_target: Option<usize>,
}

#[derive(Clone)]
struct OuterScope {
    bindings: HashMap<JsString, Binding>,
    /// Distance from the closure's environment (the parent's current env)
    /// to this scope's environment.
    offset: u32,
}

struct FnState {
    proto: FunctionProto,
    scopes: Vec<Scope>,
    current_scope: usize,
    depth: u32,
    temps: u32,
    current_handler: Option<u32>,
    loops: Vec<LoopCtx>,
    used_arguments: bool,
}

impl FnState {
    fn new(kind: ProtoKind, name: JsString) -> Self {
        FnState {
            proto: FunctionProto::new(name, kind),
            scopes: Vec::new(),
            current_scope: 0,
            depth: 0,
            temps: 0,
            current_handler: None,
            loops: Vec::new(),
            used_arguments: false,
        }
    }
}

struct Compiler<'a> {
    source: &'a str,
    strict: bool,
    state: FnState,
    outer_scopes: Vec<OuterScope>,
    eval_mode: bool,
    private_names: HashMap<JsString, SymbolValue>,
    with_depth: u32,
}

static PRIVATE_COUNTER: AtomicU64 = AtomicU64::new(10_000_000);

fn new_private_symbol(description: &JsString) -> SymbolValue {
    SymbolValue {
        id: PRIVATE_COUNTER.fetch_add(1, Ordering::Relaxed),
        description: Some(description.clone()),
    }
}

impl<'a> Compiler<'a> {
    fn new(source: &'a str, strict: bool, kind: ProtoKind) -> Self {
        Compiler {
            source,
            strict,
            state: FnState::new(kind, JsString::empty()),
            outer_scopes: Vec::new(),
            eval_mode: false,
            private_names: HashMap::new(),
            with_depth: 0,
        }
    }

    /// Snapshot the lexical scopes visible at the current point, with offsets
    /// relative to the current environment (which a nested closure captures).
    fn capture_outer(&self) -> Vec<OuterScope> {
        let mut out = Vec::new();
        let cur_depth = self.state.depth;
        let mut idx = Some(self.state.current_scope);
        while let Some(i) = idx {
            let scope = &self.state.scopes[i];
            out.push(OuterScope {
                bindings: scope.bindings.clone(),
                offset: cur_depth - scope.depth,
            });
            idx = scope.parent;
        }
        for outer in &self.outer_scopes {
            out.push(OuterScope {
                bindings: outer.bindings.clone(),
                offset: cur_depth + 1 + outer.offset,
            });
        }
        out
    }

    fn finish(self) -> FunctionProto {
        self.state.proto
    }

    fn scope(&self) -> &Scope {
        &self.state.scopes[self.state.current_scope]
    }

    fn scope_mut(&mut self) -> &mut Scope {
        let i = self.state.current_scope;
        &mut self.state.scopes[i]
    }

    fn push_scope(&mut self, kind: ScopeKind) -> usize {
        let parent = self.state.current_scope;
        let depth = self.state.depth + 1;
        let idx = self.state.scopes.len();
        self.state.scopes.push(Scope {
            parent: Some(parent),
            depth,
            kind,
            slots: Vec::new(),
            next_slot: 0,
            bindings: HashMap::new(),
        });
        self.state.current_scope = idx;
        self.state.depth = depth;
        idx
    }

    fn pop_scope(&mut self) {
        if let Some(parent) = self.state.scopes[self.state.current_scope].parent {
            let slots = self.state.scopes[self.state.current_scope].next_slot;
            let _ = slots;
            self.state.current_scope = parent;
            self.state.depth = self.state.scopes[parent].depth;
        }
    }

    fn alloc_slot(&mut self) -> u16 {
        let slot = self.scope().next_slot;
        self.scope_mut().next_slot += 1;
        self.state.scopes[self.state.current_scope].slots.push(slot);
        slot
    }

    fn declare(&mut self, name: JsString, is_const: bool) -> u16 {
        if let Some(b) = self.scope().bindings.get(&name) {
            return b.slot;
        }
        let slot = self.alloc_slot();
        self.scope_mut()
            .bindings
            .insert(name, Binding { slot, is_const });
        slot
    }

    fn find_binding(&self, name: &JsString) -> Option<(usize, u16, bool)> {
        let mut idx = Some(self.state.current_scope);
        while let Some(i) = idx {
            let scope = &self.state.scopes[i];
            if scope.kind != ScopeKind::Script || scope.kind == ScopeKind::Script {
                if let Some(b) = scope.bindings.get(name) {
                    return Some((i, b.slot, b.is_const));
                }
            }
            if scope.kind == ScopeKind::Function || scope.kind == ScopeKind::Script {
                break;
            }
            idx = scope.parent;
        }
        None
    }

    fn resolve_local(&self, name: &JsString) -> Option<(u16, u16, bool)> {
        if let Some((scope_idx, slot, is_const)) = self.find_binding(name) {
            let target_depth = self.state.scopes[scope_idx].depth;
            let depth = self.state.scopes[self.state.current_scope]
                .depth
                .saturating_sub(target_depth);
            return Some((depth as u16, slot, is_const));
        }
        let cur_depth = self.state.scopes[self.state.current_scope].depth;
        for outer in &self.outer_scopes {
            if let Some(b) = outer.bindings.get(name) {
                let depth = cur_depth + 1 + outer.offset;
                return Some((depth as u16, b.slot, b.is_const));
            }
        }
        None
    }

    fn emit(&mut self, op: Op) -> usize {
        let pos = self.state.proto.body.len();
        self.state.proto.body.push(op);
        pos
    }

    fn emit_const(&mut self, c: Constant) -> u32 {
        let idx = self.state.proto.constants.len() as u32;
        self.state.proto.constants.push(c);
        idx
    }

    fn emit_push_const(&mut self, c: Constant) {
        let idx = self.emit_const(c);
        self.emit(Op::PushConst(idx));
    }

    fn emit_string_const(&mut self, s: &str) -> u32 {
        self.emit_const(Constant::String(JsString::from_str(s)))
    }

    fn emit_name_const(&mut self, s: &JsString) -> u32 {
        self.emit_const(Constant::String(s.clone()))
    }

    fn emit_jump(&mut self, op: Op) -> usize {
        self.emit(op)
    }

    fn patch_jump(&mut self, pos: usize, target: usize) {
        let target = target as u32;
        match &mut self.state.proto.body[pos] {
            Op::Jump(t)
            | Op::JumpIfFalse(t)
            | Op::JumpIfTrue(t)
            | Op::JumpIfFalsePop(t)
            | Op::JumpIfTruePop(t)
            | Op::JumpIfNullishPop(t)
            | Op::JumpIfNotNullishPop(t)
            | Op::JumpIfNotUndefined(t) => *t = target,
            Op::IterNext { done, .. } => *done = target,
            Op::IterNextDestructure { done, .. } => *done = target,
            Op::ForInNext { done, .. } => *done = target,
            _ => panic!("not a jump op"),
        }
    }

    fn alloc_temp(&mut self) -> u32 {
        let t = self.state.temps;
        self.state.temps += 1;
        t
    }

    fn enter_loop(&mut self, label: Option<JsString>, is_loop: bool) {
        self.state.loops.push(LoopCtx {
            label,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            scope_depth: self.state.depth,
            is_loop,
            continue_target: None,
        });
    }

    fn leave_loop(&mut self, end: usize, continue_target: Option<usize>) {
        let ctx = self.state.loops.pop().unwrap();
        for j in ctx.break_jumps {
            self.patch_jump(j, end);
        }
        for j in ctx.continue_jumps {
            self.patch_jump(j, continue_target.unwrap_or(end));
        }
    }

    fn find_loop_label(&self, label: &Option<JsString>) -> Option<usize> {
        match label {
            Some(l) => self
                .state
                .loops
                .iter()
                .rposition(|c| c.label.as_ref() == Some(l)),
            None => self.state.loops.iter().rposition(|c| c.is_loop),
        }
    }

    // ------------------------------------------------------------------
    // Program / function setup
    // ------------------------------------------------------------------
    fn setup_scope(&mut self, kind: ScopeKind) {
        self.state.current_scope = 0;
        self.state.scopes.push(Scope {
            parent: None,
            depth: 0,
            kind,
            slots: Vec::new(),
            next_slot: 0,
            bindings: HashMap::new(),
        });
    }

    fn compile_script_program(&mut self, program: &Program) -> CResult<()> {
        self.setup_scope(ScopeKind::Script);
        self.state.proto.kind = ProtoKind::Script;
        self.state.proto.is_strict = self.strict;
        self.state.proto.name = JsString::from_str("script");

        let body: Vec<&Statement> = program.body.iter().collect();
        // Global var declarations.
        let mut global_vars = Vec::new();
        for stmt in &body {
            collect_global_var_names(stmt, &mut global_vars);
        }
        for name in &global_vars {
            let idx = self.emit_name_const(name);
            self.emit(Op::DeclareGlobalVar(idx));
        }
        // Top-level lexical declarations (functions are global properties).
        for stmt in &body {
            match stmt {
                Statement::VariableDeclaration(v) => {
                    if v.kind == VariableDeclarationKind::Let
                        || v.kind == VariableDeclarationKind::Const
                    {
                        for d in &v.declarations {
                            self.predeclare_binding_pattern(&d.id);
                        }
                    }
                }
                Statement::ClassDeclaration(c) => {
                    if let Some(id) = &c.id {
                        self.declare(JsString::from_str(id.name.as_str()), true);
                    }
                }
                _ => {}
            }
        }
        // Top-level function declarations.
        for stmt in &body {
            if let Statement::FunctionDeclaration(func) = stmt {
                let name = func
                    .id
                    .as_ref()
                    .map(|i| JsString::from_str(i.name.as_str()))
                    .unwrap_or_default();
                let proto =
                    self.compile_function_node(&**func, ProtoKind::Function, name, self.strict)?;
                let fpos = self.add_nested_function(proto);
                self.emit(Op::CreateClosure(fpos));
                let idx = self.emit_name_const(
                    &func
                        .id
                        .as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default(),
                );
                self.emit(Op::StoreGlobalFunction(idx));
            }
        }
        // Script-level lexical declarations are allocated as slots; the script
        // env is pre-created by the VM.
        self.state.proto.env_slots = self.scope().next_slot as u32;
        if self.eval_mode {
            let n = body.len();
            for (i, stmt) in body.iter().enumerate() {
                if i + 1 == n {
                    if let Statement::ExpressionStatement(e) = stmt {
                        self.compile_expression(&e.expression)?;
                        self.emit(Op::Return);
                        self.state.proto.temp_slots = self.state.temps;
                        return Ok(());
                    }
                }
                self.compile_statement(stmt)?;
            }
            self.emit(Op::ReturnUndefined);
            self.state.proto.temp_slots = self.state.temps;
            return Ok(());
        }
        for stmt in &body {
            self.compile_statement(stmt)?;
        }
        self.emit(Op::ReturnUndefined);
        self.state.proto.temp_slots = self.state.temps;
        Ok(())
    }

    fn compile_function_node(
        &mut self,
        func: &Function,
        kind: ProtoKind,
        name: JsString,
        inherited_strict: bool,
    ) -> CResult<Arc<FunctionProto>> {
        let body = match &func.body {
            Some(b) => b,
            None => return Err(CompileError::new("function has no body")),
        };
        let mut strict = inherited_strict
            || body
                .directives
                .iter()
                .any(|d| d.directive.as_str() == "use strict")
            || matches!(
                kind,
                ProtoKind::ClassConstructor | ProtoKind::DerivedConstructor | ProtoKind::Method
            );
        if func.generator && func.r#async {
            // Async generators use the async generator prototype.
        }
        let kind = if func.generator {
            if func.r#async {
                ProtoKind::AsyncGenerator
            } else {
                ProtoKind::Generator
            }
        } else if func.r#async {
            ProtoKind::Async
        } else {
            kind
        };
        strict = strict || self.strict;
        let outer = self.capture_outer();
        let mut c = Compiler::new(self.source, strict, kind);
        c.outer_scopes = outer;
        c.private_names = self.private_names.clone();
        c.state.proto.name = name.clone();
        c.state.proto.is_strict = strict;
        if func.generator || func.r#async {
            // Generators/async functions compile their bodies like normal
            // functions; the VM turns calls into generator/async objects.
        }
        c.setup_scope(ScopeKind::Function);
        // Parameters.
        let mut simple_count = 0u32;
        let mut seen_default = false;
        for item in &func.params.items {
            if item.initializer.is_some() {
                seen_default = true;
            }
            if !seen_default {
                simple_count += 1;
            }
            c.predeclare_binding_pattern(&item.pattern);
        }
        if let Some(rest) = &func.params.rest {
            c.predeclare_binding_pattern(&rest.rest.argument);
        }
        c.state.proto.length = simple_count;
        // arguments slot.
        c.declare(JsString::from_str("arguments"), false);
        c.state.used_arguments = true;
        // Collect var declarations.
        let mut var_names = Vec::new();
        for stmt in &body.statements {
            collect_var_names(stmt, &mut var_names);
        }
        for name in &var_names {
            c.declare(name.clone(), false);
        }
        // Top-level function declarations share the function scope.
        for stmt in &body.statements {
            if let Statement::FunctionDeclaration(f) = stmt {
                let n =
                    f.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                c.declare(n, false);
            }
        }
        let init_bindings = c.snapshot_bindings();
        // Top-level lexical declarations.
        for stmt in &body.statements {
            c.predeclare_lexical(stmt, true);
        }
        c.state.proto.env_slots = c.scope().next_slot as u32;
        // Initialize non-parameter var slots to undefined.
        let param_slots = c.param_slot_names(func);
        for (name, binding_slot) in &init_bindings {
            if !param_slots.contains(name) {
                c.emit(Op::PushUndefined);
                let (depth, slot) = c.binding_location(name).unwrap_or((0, *binding_slot));
                c.emit(Op::InitVar { depth, slot });
            }
        }
        // Bind parameters.
        for (i, item) in func.params.items.iter().enumerate() {
            c.emit(Op::LoadArg(i as u32));
            if let Some(init) = &item.initializer {
                let skip = c.emit(Op::JumpIfNotUndefined(0));
                c.emit(Op::Pop);
                c.compile_expression(init)?;
                let here = c.state.proto.body.len();
                c.patch_jump(skip, here);
            }
            c.bind_binding_pattern(&item.pattern, true)?;
        }
        if let Some(rest) = &func.params.rest {
            c.emit(Op::CreateRestArray(func.params.items.len() as u32));
            c.bind_binding_pattern(&rest.rest.argument, true)?;
        }
        // arguments object.
        c.emit(Op::CreateArguments);
        if let Some((depth, slot, _)) = c.resolve_local(&JsString::from_str("arguments")) {
            c.emit(Op::InitVar { depth, slot });
        } else {
            c.emit(Op::Pop);
        }
        // Hoist function declarations at the body top.
        for stmt in &body.statements {
            if let Statement::FunctionDeclaration(f) = stmt {
                let n =
                    f.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                let proto =
                    c.compile_function_node(&**f, ProtoKind::Function, n.clone(), strict)?;
                let idx = c.add_nested_function(proto);
                c.emit(Op::CreateClosure(idx));
                if let Some((depth, slot, _)) = c.resolve_local(&n) {
                    c.emit(Op::InitVar { depth, slot });
                } else {
                    c.emit(Op::Pop);
                }
            }
        }
        for stmt in &body.statements {
            c.compile_statement(stmt)?;
        }
        c.emit(Op::ReturnUndefined);
        c.state.proto.temp_slots = c.state.temps;
        c.state.proto.source_text = Some(JsString::from_str(
            &self.source[func.span.start as usize..func.span.end as usize],
        ));
        Ok(Arc::new(c.finish()))
    }

    fn param_slot_names(&self, func: &Function) -> Vec<JsString> {
        let mut v = Vec::new();
        for item in &func.params.items {
            collect_pattern_names(&item.pattern, &mut v);
        }
        if let Some(rest) = &func.params.rest {
            collect_pattern_names(&rest.rest.argument, &mut v);
        }
        v
    }

    fn snapshot_bindings(&self) -> Vec<(JsString, u16)> {
        self.scope()
            .bindings
            .iter()
            .map(|(k, v)| (k.clone(), v.slot))
            .collect()
    }

    fn binding_location(&self, name: &JsString) -> Option<(u16, u16)> {
        self.resolve_local(name).map(|(d, s, _)| (d, s))
    }

    fn add_nested_function(&mut self, proto: Arc<FunctionProto>) -> u32 {
        let idx = self.state.proto.functions.len() as u32;
        self.state.proto.functions.push(proto);
        idx
    }

    fn compile_arrow(
        &mut self,
        arrow: &ArrowFunctionExpression,
        name: JsString,
        parent_strict: bool,
    ) -> CResult<Arc<FunctionProto>> {
        let (body_stmts, body_expr, strict_body) = match &arrow.body {
            ArrowFunctionBody::FunctionBody(b) => {
                let strict = b
                    .directives
                    .iter()
                    .any(|d| d.directive.as_str() == "use strict");
                (Some(&b.statements), None, strict)
            }
            other => match other.as_expression() {
                Some(e) => (None, Some(e), false),
                None => return Err(CompileError::new("invalid arrow body")),
            },
        };
        let strict = parent_strict || strict_body;
        let outer = self.capture_outer();
        let arrow_kind = if arrow.r#async {
            ProtoKind::Async
        } else {
            ProtoKind::Arrow
        };
        let mut c = Compiler::new(self.source, strict, arrow_kind);
        c.outer_scopes = outer;
        c.private_names = self.private_names.clone();
        c.state.proto.is_arrow = true;
        c.state.proto.name = name.clone();
        c.state.proto.is_strict = strict;
        c.setup_scope(ScopeKind::Function);
        let mut simple_count = 0u32;
        let mut seen_default = false;
        for item in &arrow.params.items {
            if item.initializer.is_some() {
                seen_default = true;
            }
            if !seen_default {
                simple_count += 1;
            }
            c.predeclare_binding_pattern(&item.pattern);
        }
        if let Some(rest) = &arrow.params.rest {
            c.predeclare_binding_pattern(&rest.rest.argument);
        }
        c.state.proto.length = simple_count;
        c.declare(JsString::from_str("arguments"), false);
        let mut init_bindings = Vec::new();
        if let Some(stmts) = body_stmts {
            let mut var_names = Vec::new();
            for stmt in stmts {
                collect_var_names(stmt, &mut var_names);
            }
            for n in var_names {
                c.declare(n, false);
            }
            for stmt in stmts {
                if let Statement::FunctionDeclaration(f) = stmt {
                    let n =
                        f.id.as_ref()
                            .map(|i| JsString::from_str(i.name.as_str()))
                            .unwrap_or_default();
                    c.declare(n, false);
                }
            }
            init_bindings = c.snapshot_bindings();
            for stmt in stmts {
                c.predeclare_lexical(stmt, true);
            }
        }
        c.state.proto.env_slots = c.scope().next_slot as u32;
        let param_names = {
            let mut v = Vec::new();
            for item in &arrow.params.items {
                collect_pattern_names(&item.pattern, &mut v);
            }
            if let Some(rest) = &arrow.params.rest {
                collect_pattern_names(&rest.rest.argument, &mut v);
            }
            v
        };
        for (name, _) in &init_bindings {
            if !param_names.contains(name) {
                c.emit(Op::PushUndefined);
                if let Some((depth, slot)) = c.binding_location(name) {
                    c.emit(Op::InitVar { depth, slot });
                }
            }
        }
        for (i, item) in arrow.params.items.iter().enumerate() {
            c.emit(Op::LoadArg(i as u32));
            if let Some(init) = &item.initializer {
                let skip = c.emit(Op::JumpIfNotUndefined(0));
                c.emit(Op::Pop);
                c.compile_expression(init)?;
                let here = c.state.proto.body.len();
                c.patch_jump(skip, here);
            }
            c.bind_binding_pattern(&item.pattern, true)?;
        }
        if let Some(rest) = &arrow.params.rest {
            c.emit(Op::CreateRestArray(arrow.params.items.len() as u32));
            c.bind_binding_pattern(&rest.rest.argument, true)?;
        }
        if let Some(stmts) = body_stmts {
            for stmt in stmts {
                if let Statement::FunctionDeclaration(f) = stmt {
                    let n =
                        f.id.as_ref()
                            .map(|i| JsString::from_str(i.name.as_str()))
                            .unwrap_or_default();
                    let proto =
                        c.compile_function_node(&**f, ProtoKind::Function, n.clone(), strict)?;
                    let idx = c.add_nested_function(proto);
                    c.emit(Op::CreateClosure(idx));
                    if let Some((depth, slot)) = c.binding_location(&n) {
                        c.emit(Op::InitVar { depth, slot });
                    } else {
                        c.emit(Op::Pop);
                    }
                }
            }
            for stmt in stmts {
                c.compile_statement(stmt)?;
            }
            c.emit(Op::ReturnUndefined);
        } else {
            c.compile_expression(body_expr.unwrap())?;
            c.emit(Op::Return);
        }
        c.state.proto.temp_slots = c.state.temps;
        c.state.proto.source_text = Some(JsString::from_str(
            &self.source[arrow.span.start as usize..arrow.span.end as usize],
        ));
        Ok(Arc::new(c.finish()))
    }

    // ------------------------------------------------------------------
    // Declarations
    // ------------------------------------------------------------------
    fn predeclare_binding_pattern(&mut self, pattern: &BindingPattern) {
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                self.declare(JsString::from_str(id.name.as_str()), false);
            }
            BindingPattern::ObjectPattern(o) => {
                for p in &o.properties {
                    self.predeclare_binding_pattern(&p.value);
                }
                if let Some(rest) = &o.rest {
                    self.predeclare_binding_pattern(&rest.argument);
                }
            }
            BindingPattern::ArrayPattern(a) => {
                for e in &a.elements {
                    if let Some(e) = e {
                        self.predeclare_binding_pattern(e);
                    }
                }
                if let Some(rest) = &a.rest {
                    self.predeclare_binding_pattern(&rest.argument);
                }
            }
            BindingPattern::AssignmentPattern(a) => {
                self.predeclare_binding_pattern(&a.left);
            }
        }
    }

    fn predeclare_lexical(&mut self, stmt: &Statement, _function_body: bool) {
        match stmt {
            Statement::VariableDeclaration(v) => {
                if v.kind == VariableDeclarationKind::Let
                    || v.kind == VariableDeclarationKind::Const
                {
                    for d in &v.declarations {
                        self.predeclare_binding_pattern(&d.id);
                    }
                }
            }
            Statement::ClassDeclaration(c) => {
                if let Some(id) = &c.id {
                    self.declare(JsString::from_str(id.name.as_str()), true);
                }
            }
            Statement::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    self.declare(JsString::from_str(id.name.as_str()), false);
                }
            }
            _ => {}
        }
    }

    fn bind_binding_pattern(&mut self, pattern: &BindingPattern, init: bool) -> CResult<()> {
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                if let Some((depth, slot, is_const)) = self.resolve_local(&name) {
                    if init {
                        self.emit(Op::InitVar { depth, slot });
                    } else if is_const {
                        self.emit(Op::ThrowConstAssignment);
                    } else {
                        self.emit(Op::StoreVar { depth, slot });
                        self.emit(Op::Pop);
                    }
                } else {
                    let idx = self.emit_name_const(&name);
                    if init {
                        self.emit(Op::StoreGlobal {
                            name: idx,
                            strict: self.strict,
                        });
                        self.emit(Op::Pop);
                    } else {
                        self.emit(Op::StoreGlobal {
                            name: idx,
                            strict: self.strict,
                        });
                        self.emit(Op::Pop);
                    }
                }
            }
            BindingPattern::AssignmentPattern(a) => {
                let skip = self.emit(Op::JumpIfNotUndefined(0));
                self.emit(Op::Pop);
                self.compile_expression(&a.right)?;
                let here = self.state.proto.body.len();
                self.patch_jump(skip, here);
                self.bind_binding_pattern(&a.left, init)?;
            }
            BindingPattern::ObjectPattern(o) => {
                let src = self.alloc_temp();
                self.emit(Op::StoreTemp(src));
                let mut excluded = Vec::new();
                for p in &o.properties {
                    self.emit(Op::LoadTemp(src));
                    let (computed, key_idx) = self.compile_property_key(&p.key)?;
                    if computed {
                        self.emit(Op::GetElem);
                    } else {
                        self.emit(Op::GetProp(key_idx));
                    }
                    if !computed {
                        // GetProp already used the key; recover key for exclusion.
                    }
                    if let Some(name) = property_key_name(&p.key) {
                        excluded.push(name);
                    }
                    self.bind_binding_pattern(&p.value, init)?;
                }
                if let Some(rest) = &o.rest {
                    let count = excluded.len() as u32;
                    self.emit(Op::NewObject);
                    self.emit(Op::LoadTemp(src));
                    for name in &excluded {
                        let idx = self.emit_name_const(name);
                        self.emit(Op::PushConst(idx));
                    }
                    self.emit(Op::CopyDataPropertiesExcluding(count));
                    self.bind_binding_pattern(&rest.argument, init)?;
                }
            }
            BindingPattern::ArrayPattern(a) => {
                let src = self.alloc_temp();
                let iter = self.alloc_temp();
                self.emit(Op::StoreTemp(src));
                self.emit(Op::LoadTemp(src));
                self.emit(Op::IterInit(iter));
                for e in &a.elements {
                    if e.is_none() {
                        let skip = self.emit(Op::IterNextDestructure {
                            temp: iter,
                            done: 0,
                        });
                        self.emit(Op::Pop);
                        let after = self.state.proto.body.len();
                        self.patch_jump(skip, after);
                        continue;
                    }
                    let skip = self.emit(Op::IterNextDestructure {
                        temp: iter,
                        done: 0,
                    });
                    let cont = self.emit(Op::Jump(0));
                    let done_pos = self.state.proto.body.len();
                    self.patch_jump(skip, done_pos);
                    self.emit(Op::PushUndefined);
                    let after_done = self.state.proto.body.len();
                    self.patch_jump(cont, after_done);
                    self.bind_binding_pattern(e.as_ref().unwrap(), init)?;
                }
                if let Some(rest) = &a.rest {
                    self.emit(Op::IterRest { temp: iter });
                    self.bind_binding_pattern(&rest.argument, init)?;
                }
                self.emit(Op::IterClose { temp: iter });
            }
        }
        Ok(())
    }

    fn compile_property_key(&mut self, key: &PropertyKey) -> CResult<(bool, u32)> {
        match key {
            PropertyKey::StaticIdentifier(id) => {
                let idx = self.emit_name_const(&JsString::from_str(id.name.as_str()));
                Ok((false, idx))
            }
            PropertyKey::PrivateIdentifier(id) => {
                let n = JsString::from_str(id.name.as_str());
                let sym = self
                    .private_names
                    .get(&n)
                    .cloned()
                    .unwrap_or_else(|| new_private_symbol(&n));
                self.emit_push_const(Constant::Symbol(sym));
                Ok((true, 0))
            }
            other => {
                if let Some(e) = other.as_expression() {
                    self.compile_expression(e)?;
                    Ok((true, 0))
                } else {
                    Err(CompileError::new("unsupported property key"))
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------
    fn compile_statement(&mut self, stmt: &Statement) -> CResult<()> {
        match stmt {
            Statement::EmptyStatement(_) => {}
            Statement::DebuggerStatement(_) => {}
            Statement::ExpressionStatement(e) => {
                self.compile_expression(&e.expression)?;
                self.emit(Op::Pop);
            }
            Statement::BlockStatement(b) => {
                self.compile_block(&b.body, true)?;
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    if v.kind == VariableDeclarationKind::Var {
                        if let Some(init) = &d.init {
                            self.compile_expression(init)?;
                            self.bind_binding_pattern(&d.id, false)?;
                        }
                    } else {
                        if let Some(init) = &d.init {
                            self.compile_expression(init)?;
                        } else {
                            self.emit(Op::PushUndefined);
                        }
                        self.bind_binding_pattern(&d.id, true)?;
                    }
                }
            }
            Statement::FunctionDeclaration(_) => {
                // Hoisted at scope entry.
            }
            Statement::ClassDeclaration(c) => {
                let name =
                    c.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                self.compile_class(c, name, true)?;
                if let Some((depth, slot, _)) = self.resolve_local(
                    &c.id
                        .as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default(),
                ) {
                    self.emit(Op::InitVar { depth, slot });
                } else {
                    self.emit(Op::Pop);
                }
            }
            Statement::IfStatement(i) => {
                self.compile_expression(&i.test)?;
                let else_jump = self.emit(Op::JumpIfFalse(0));
                self.compile_statement(&i.consequent)?;
                if let Some(alt) = &i.alternate {
                    let end_jump = self.emit(Op::Jump(0));
                    let else_pos = self.state.proto.body.len();
                    self.patch_jump(else_jump, else_pos);
                    self.compile_statement(alt)?;
                    let end = self.state.proto.body.len();
                    self.patch_jump(end_jump, end);
                } else {
                    let end = self.state.proto.body.len();
                    self.patch_jump(else_jump, end);
                }
            }
            Statement::WhileStatement(w) => {
                let start = self.state.proto.body.len();
                self.enter_loop(None, true);
                self.compile_expression(&w.test)?;
                let exit = self.emit(Op::JumpIfFalse(0));
                self.compile_statement(&w.body)?;
                let cont = self.state.proto.body.len();
                self.emit(Op::Jump(start as u32));
                let end = self.state.proto.body.len();
                self.patch_jump(exit, end);
                self.leave_loop(end, Some(cont));
            }
            Statement::DoWhileStatement(d) => {
                let start = self.state.proto.body.len();
                self.enter_loop(None, true);
                self.compile_statement(&d.body)?;
                let cont = self.state.proto.body.len();
                self.compile_expression(&d.test)?;
                self.emit(Op::JumpIfTrue(start as u32));
                let end = self.state.proto.body.len();
                self.leave_loop(end, Some(cont));
            }
            Statement::ForStatement(f) => {
                // Loop-head lexical declarations live in their own env.
                let mut created_scope = false;
                if let Some(init) = &f.init {
                    if let ForStatementInit::VariableDeclaration(v) = init {
                        if v.kind != VariableDeclarationKind::Var {
                            self.push_scope(ScopeKind::Block);
                            self.emit(Op::CreateEnv(0));
                            created_scope = true;
                            for d in &v.declarations {
                                self.predeclare_binding_pattern(&d.id);
                            }
                            self.state.proto.env_slots = self.state.proto.env_slots.max(0);
                            let slots = self.scope().next_slot;
                            self.state.proto.body.pop();
                            self.emit(Op::CreateEnv(slots as u32));
                        }
                    }
                }
                if let Some(init) = &f.init {
                    match init {
                        ForStatementInit::VariableDeclaration(v) => {
                            if v.kind == VariableDeclarationKind::Var {
                                for d in &v.declarations {
                                    if let Some(init) = &d.init {
                                        self.compile_expression(init)?;
                                        self.bind_binding_pattern(&d.id, false)?;
                                    }
                                }
                            } else {
                                for d in &v.declarations {
                                    if let Some(init) = &d.init {
                                        self.compile_expression(init)?;
                                    } else {
                                        self.emit(Op::PushUndefined);
                                    }
                                    self.bind_binding_pattern(&d.id, true)?;
                                }
                            }
                        }
                        other => {
                            if let Some(e) = other.as_expression() {
                                self.compile_expression(e)?;
                                self.emit(Op::Pop);
                            }
                        }
                    }
                }
                let test_pos = self.state.proto.body.len();
                let exit = if let Some(test) = &f.test {
                    self.compile_expression(test)?;
                    Some(self.emit(Op::JumpIfFalse(0)))
                } else {
                    None
                };
                self.enter_loop(None, true);
                self.compile_statement(&f.body)?;
                let cont = self.state.proto.body.len();
                if let Some(update) = &f.update {
                    self.compile_expression(update)?;
                    self.emit(Op::Pop);
                }
                self.emit(Op::Jump(test_pos as u32));
                let end = self.state.proto.body.len();
                if let Some(e) = exit {
                    self.patch_jump(e, end);
                }
                self.leave_loop(end, Some(cont));
                if created_scope {
                    self.emit(Op::PopEnv);
                    self.pop_scope();
                }
            }
            Statement::ForInStatement(f) => self.compile_for_in(f, false)?,
            Statement::ForOfStatement(f) => self.compile_for_of(f, f.r#await)?,
            Statement::BreakStatement(b) => {
                let label = b
                    .label
                    .as_ref()
                    .map(|l| JsString::from_str(l.name.as_str()));
                let idx = match &label {
                    Some(l) => self
                        .state
                        .loops
                        .iter()
                        .rposition(|c| c.label.as_ref() == Some(l)),
                    None => self.state.loops.len().checked_sub(1),
                }
                .ok_or_else(|| CompileError::new("break label not found"))?;
                let target_depth = self.state.loops[idx].scope_depth;
                let cur = self.state.depth;
                for _ in target_depth..cur {
                    self.emit(Op::PopEnv);
                }
                let jump = self.emit(Op::Jump(0));
                self.state.loops[idx].break_jumps.push(jump);
            }
            Statement::ContinueStatement(c) => {
                let label = c
                    .label
                    .as_ref()
                    .map(|l| JsString::from_str(l.name.as_str()));
                let idx = match &label {
                    Some(l) => self
                        .state
                        .loops
                        .iter()
                        .rposition(|c| c.label.as_ref() == Some(l)),
                    None => self.state.loops.iter().rposition(|c| c.is_loop),
                }
                .ok_or_else(|| CompileError::new("continue label not found"))?;
                if !self.state.loops[idx].is_loop {
                    return Err(CompileError::new("continue target is not a loop"));
                }
                let target_depth = self.state.loops[idx].scope_depth;
                let cur = self.state.depth;
                for _ in target_depth..cur {
                    self.emit(Op::PopEnv);
                }
                let jump = self.emit(Op::Jump(0));
                self.state.loops[idx].continue_jumps.push(jump);
            }
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &r.argument {
                    self.compile_expression(arg)?;
                } else {
                    self.emit(Op::PushUndefined);
                }
                self.emit(Op::Return);
            }
            Statement::ThrowStatement(t) => {
                self.compile_expression(&t.argument)?;
                self.emit(Op::Throw);
            }
            Statement::LabeledStatement(l) => {
                let label = Some(JsString::from_str(l.label.name.as_str()));
                let is_loop = matches!(
                    &l.body,
                    Statement::WhileStatement(_)
                        | Statement::DoWhileStatement(_)
                        | Statement::ForStatement(_)
                        | Statement::ForInStatement(_)
                        | Statement::ForOfStatement(_)
                );
                if is_loop {
                    self.enter_loop(label, true);
                    self.compile_statement_inner_loop(&l.body)?;
                    let end = self.state.proto.body.len();
                    self.leave_loop(end, None);
                } else {
                    self.enter_loop(label, false);
                    self.compile_statement(&l.body)?;
                    let end = self.state.proto.body.len();
                    self.leave_loop(end, None);
                }
            }
            Statement::SwitchStatement(s) => {
                self.compile_switch(s)?;
            }
            Statement::TryStatement(t) => {
                self.compile_try(t)?;
            }
            Statement::WithStatement(w) => {
                self.compile_expression(&w.object)?;
                self.emit(Op::CreateWithEnv);
                self.push_scope(ScopeKind::Block);
                self.with_depth += 1;
                self.compile_statement(&w.body)?;
                self.with_depth -= 1;
                self.emit(Op::PopEnv);
                self.pop_scope();
            }
            _ => {
                if let Some(decl) = stmt.as_declaration() {
                    match decl {
                        Declaration::ClassDeclaration(c) => {
                            let name =
                                c.id.as_ref()
                                    .map(|i| JsString::from_str(i.name.as_str()))
                                    .unwrap_or_default();
                            self.compile_class(c, name.clone(), true)?;
                            if let Some((depth, slot, _)) = self.resolve_local(&name) {
                                self.emit(Op::InitVar { depth, slot });
                            } else {
                                self.emit(Op::Pop);
                            }
                        }
                        _ => return Err(CompileError::new("unsupported declaration")),
                    }
                } else {
                    return Err(CompileError::new("unsupported statement"));
                }
            }
        }
        Ok(())
    }

    fn compile_statement_inner_loop(&mut self, stmt: &Statement) -> CResult<()> {
        // Compile a loop without pushing a second loop context (labeled loop).
        match stmt {
            Statement::WhileStatement(w) => {
                let start = self.state.proto.body.len();
                self.compile_expression(&w.test)?;
                let exit = self.emit(Op::JumpIfFalse(0));
                self.compile_statement(&w.body)?;
                let cont = self.state.proto.body.len();
                self.emit(Op::Jump(start as u32));
                let end = self.state.proto.body.len();
                self.patch_jump(exit, end);
                let (cont_jumps, break_jumps) = match self.state.loops.last_mut() {
                    Some(ctx) => (
                        std::mem::take(&mut ctx.continue_jumps),
                        std::mem::take(&mut ctx.break_jumps),
                    ),
                    None => (Vec::new(), Vec::new()),
                };
                for j in cont_jumps {
                    self.patch_jump(j, cont);
                }
                for j in break_jumps {
                    self.patch_jump(j, end);
                }
            }
            Statement::ForStatement(f) => {
                if let Some(init) = &f.init {
                    match init {
                        ForStatementInit::VariableDeclaration(v) => {
                            for d in &v.declarations {
                                if let Some(init) = &d.init {
                                    self.compile_expression(init)?;
                                    self.bind_binding_pattern(&d.id, false)?;
                                }
                            }
                        }
                        other => {
                            if let Some(e) = other.as_expression() {
                                self.compile_expression(e)?;
                                self.emit(Op::Pop);
                            }
                        }
                    }
                }
                let test_pos = self.state.proto.body.len();
                let exit = if let Some(test) = &f.test {
                    self.compile_expression(test)?;
                    Some(self.emit(Op::JumpIfFalse(0)))
                } else {
                    None
                };
                self.compile_statement(&f.body)?;
                let cont = self.state.proto.body.len();
                if let Some(update) = &f.update {
                    self.compile_expression(update)?;
                    self.emit(Op::Pop);
                }
                self.emit(Op::Jump(test_pos as u32));
                let end = self.state.proto.body.len();
                if let Some(e) = exit {
                    self.patch_jump(e, end);
                }
                let (cont_jumps, break_jumps) = match self.state.loops.last_mut() {
                    Some(ctx) => (
                        std::mem::take(&mut ctx.continue_jumps),
                        std::mem::take(&mut ctx.break_jumps),
                    ),
                    None => (Vec::new(), Vec::new()),
                };
                for j in cont_jumps {
                    self.patch_jump(j, cont);
                }
                for j in break_jumps {
                    self.patch_jump(j, end);
                }
            }
            _ => self.compile_statement(stmt)?,
        }
        Ok(())
    }

    fn compile_block(&mut self, stmts: &[Statement], create_scope: bool) -> CResult<()> {
        if create_scope {
            self.push_scope(ScopeKind::Block);
            // Predeclare direct lexical declarations.
            for stmt in stmts {
                self.predeclare_lexical(stmt, false);
            }
            let slots = self.scope().next_slot;
            self.emit(Op::CreateEnv(slots as u32));
            // Hoist block function declarations.
            for stmt in stmts {
                if let Statement::FunctionDeclaration(f) = stmt {
                    let name =
                        f.id.as_ref()
                            .map(|i| JsString::from_str(i.name.as_str()))
                            .unwrap_or_default();
                    let proto = self.compile_function_node(
                        &**f,
                        ProtoKind::Function,
                        name.clone(),
                        self.strict,
                    )?;
                    let idx = self.add_nested_function(proto);
                    self.emit(Op::CreateClosure(idx));
                    if let Some((depth, slot, _)) = self.resolve_local(&name) {
                        self.emit(Op::InitVar { depth, slot });
                    } else {
                        self.emit(Op::Pop);
                    }
                }
            }
            for stmt in stmts {
                self.compile_statement(stmt)?;
            }
            self.emit(Op::PopEnv);
            self.pop_scope();
        } else {
            for stmt in stmts {
                self.compile_statement(stmt)?;
            }
        }
        Ok(())
    }

    fn compile_for_of(&mut self, f: &ForOfStatement, _async: bool) -> CResult<()> {
        self.emit(Op::CreateEnv(0));
        self.push_scope(ScopeKind::Block);
        // Declare loop variable.
        if let ForStatementLeft::VariableDeclaration(v) = &f.left {
            if v.kind != VariableDeclarationKind::Var {
                for d in &v.declarations {
                    self.predeclare_binding_pattern(&d.id);
                }
            }
            let slots = self.scope().next_slot;
            self.emit(Op::PopEnv);
            self.emit(Op::CreateEnv(slots as u32));
        }
        self.compile_expression(&f.right)?;
        if _async {
            let iter = self.alloc_temp();
            let res = self.alloc_temp();
            self.emit(Op::AsyncIterInit(iter));
            let loop_start = self.state.proto.body.len();
            self.emit(Op::AsyncIterNext(iter));
            self.emit(Op::Await);
            self.emit(Op::StoreTemp(res));
            self.emit(Op::LoadTemp(res));
            let done_key = self.emit_name_const(&JsString::from_str("done"));
            self.emit(Op::GetProp(done_key));
            let done_jump = self.emit(Op::JumpIfTrue(0));
            self.emit(Op::LoadTemp(res));
            let value_key = self.emit_name_const(&JsString::from_str("value"));
            self.emit(Op::GetProp(value_key));
            self.emit(Op::Await);
            match &f.left {
                ForStatementLeft::VariableDeclaration(v) => {
                    let init = v.kind != VariableDeclarationKind::Var;
                    if let Some(d) = v.declarations.first() {
                        self.bind_binding_pattern(&d.id, init)?;
                    }
                }
                other => {
                    if let Some(t) = other.as_assignment_target() {
                        self.bind_assignment_target(t, false)?;
                    }
                }
            }
            self.enter_loop(None, true);
            self.compile_statement(&f.body)?;
            self.emit(Op::Jump(loop_start as u32));
            self.leave_loop(self.state.proto.body.len(), Some(loop_start));
            let done_pos = self.state.proto.body.len();
            self.patch_jump(done_jump, done_pos);
            self.emit(Op::PopEnv);
            self.pop_scope();
            return Ok(());
        }
        let iter = self.alloc_temp();
        self.emit(Op::IterInit(iter));
        let loop_start = self.state.proto.body.len();
        let done_jump = self.emit(Op::IterNext {
            temp: iter,
            done: 0,
        });
        match &f.left {
            ForStatementLeft::VariableDeclaration(v) => {
                let init = v.kind != VariableDeclarationKind::Var;
                if let Some(d) = v.declarations.first() {
                    self.bind_binding_pattern(&d.id, init)?;
                }
            }
            other => {
                if let Some(t) = other.as_assignment_target() {
                    self.bind_assignment_target(t, false)?;
                }
            }
        }
        self.enter_loop(None, true);
        self.compile_statement(&f.body)?;
        self.emit(Op::Jump(loop_start as u32));
        self.leave_loop(self.state.proto.body.len(), Some(loop_start));
        let done_pos = self.state.proto.body.len();
        self.patch_jump(done_jump, done_pos);
        self.emit(Op::IterClose { temp: iter });
        self.emit(Op::PopEnv);
        self.pop_scope();
        Ok(())
    }

    fn compile_for_in(&mut self, f: &ForInStatement, _async: bool) -> CResult<()> {
        self.emit(Op::CreateEnv(0));
        self.push_scope(ScopeKind::Block);
        if let ForStatementLeft::VariableDeclaration(v) = &f.left {
            if v.kind != VariableDeclarationKind::Var {
                for d in &v.declarations {
                    self.predeclare_binding_pattern(&d.id);
                }
            }
            let slots = self.scope().next_slot;
            self.emit(Op::PopEnv);
            self.emit(Op::CreateEnv(slots as u32));
        }
        self.compile_expression(&f.right)?;
        let iter = self.alloc_temp();
        self.emit(Op::ForInInit(iter));
        let loop_start = self.state.proto.body.len();
        let done_jump = self.emit(Op::ForInNext {
            temp: iter,
            done: 0,
        });
        match &f.left {
            ForStatementLeft::VariableDeclaration(v) => {
                let init = v.kind != VariableDeclarationKind::Var;
                if let Some(d) = v.declarations.first() {
                    self.bind_binding_pattern(&d.id, init)?;
                }
            }
            other => {
                if let Some(t) = other.as_assignment_target() {
                    self.bind_assignment_target(t, false)?;
                }
            }
        }
        self.enter_loop(None, true);
        self.compile_statement(&f.body)?;
        self.emit(Op::Jump(loop_start as u32));
        self.leave_loop(self.state.proto.body.len(), Some(loop_start));
        let done_pos = self.state.proto.body.len();
        self.patch_jump(done_jump, done_pos);
        self.emit(Op::PopEnv);
        self.pop_scope();
        Ok(())
    }

    fn compile_switch(&mut self, s: &SwitchStatement) -> CResult<()> {
        self.push_scope(ScopeKind::Block);
        for case in &s.cases {
            for stmt in &case.consequent {
                self.predeclare_lexical(stmt, false);
            }
        }
        let slots = self.scope().next_slot;
        self.emit(Op::CreateEnv(slots as u32));
        for case in &s.cases {
            for stmt in &case.consequent {
                if let Statement::FunctionDeclaration(f) = stmt {
                    let name =
                        f.id.as_ref()
                            .map(|i| JsString::from_str(i.name.as_str()))
                            .unwrap_or_default();
                    let proto = self.compile_function_node(
                        &**f,
                        ProtoKind::Function,
                        name.clone(),
                        self.strict,
                    )?;
                    let idx = self.add_nested_function(proto);
                    self.emit(Op::CreateClosure(idx));
                    if let Some((depth, slot, _)) = self.resolve_local(&name) {
                        self.emit(Op::InitVar { depth, slot });
                    } else {
                        self.emit(Op::Pop);
                    }
                }
            }
        }
        let disc = self.alloc_temp();
        self.compile_expression(&s.discriminant)?;
        self.emit(Op::StoreTemp(disc));
        self.enter_loop(None, false);
        // Evaluate case tests.
        let mut case_jumps = Vec::new();
        let mut default_idx = None;
        for (i, case) in s.cases.iter().enumerate() {
            if let Some(test) = &case.test {
                self.emit(Op::LoadTemp(disc));
                self.compile_expression(test)?;
                self.emit(Op::Binary(BinOp::StrictEq));
                let j = self.emit(Op::JumpIfTrue(0));
                case_jumps.push((i, j));
            } else {
                default_idx = Some(i);
            }
        }
        let mut dispatch_end = Vec::new();
        if let Some(d) = default_idx {
            dispatch_end.push(self.emit(Op::Jump(0)));
        } else {
            dispatch_end.push(self.emit(Op::Jump(0)));
        }
        let case_positions: Vec<usize> = Vec::new();
        let _ = &case_positions;
        // Encode each case body, preceded by a dispatch label.
        let mut targets = vec![0usize; s.cases.len()];
        for (i, case) in s.cases.iter().enumerate() {
            targets[i] = self.state.proto.body.len();
            for stmt in &case.consequent {
                self.compile_statement(stmt)?;
            }
        }
        let end = self.state.proto.body.len();
        for (i, j) in case_jumps {
            self.patch_jump(j, targets[i]);
        }
        for j in dispatch_end {
            self.patch_jump(j, default_idx.map(|i| targets[i]).unwrap_or(end));
        }

        self.leave_loop(end, None);
        self.emit(Op::PopEnv);
        self.pop_scope();
        Ok(())
    }

    fn compile_try(&mut self, t: &TryStatement) -> CResult<()> {
        let outer_handler = self.state.current_handler;
        let env_depth = self.state.depth;
        let try_start = self.state.proto.body.len();
        let has_catch = t.handler.is_some();
        let has_finally = t.finalizer.is_some();

        let catch_idx = if has_catch {
            let idx = self.state.proto.handlers.len() as u32;
            self.state.proto.handlers.push(Handler {
                start: try_start as u32,
                end: 0,
                catch_target: None,
                finally_target: None,
                env_depth,
                stack_height: 0,
                outer: outer_handler,
            });
            Some(idx)
        } else {
            None
        };
        let finally_idx = if has_finally {
            let idx = self.state.proto.handlers.len() as u32;
            self.state.proto.handlers.push(Handler {
                start: try_start as u32,
                end: 0,
                catch_target: None,
                finally_target: None,
                env_depth,
                stack_height: 0,
                outer: outer_handler,
            });
            Some(idx)
        } else {
            None
        };

        self.state.current_handler = catch_idx.or(finally_idx);
        self.push_scope(ScopeKind::Block);
        for stmt in &t.block.body {
            self.predeclare_lexical(stmt, false);
        }
        let slots = self.scope().next_slot;
        self.emit(Op::CreateEnv(slots as u32));
        for stmt in &t.block.body {
            if let Statement::FunctionDeclaration(f) = stmt {
                let name =
                    f.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                let proto = self.compile_function_node(
                    &**f,
                    ProtoKind::Function,
                    name.clone(),
                    self.strict,
                )?;
                let idx = self.add_nested_function(proto);
                self.emit(Op::CreateClosure(idx));
                if let Some((depth, slot, _)) = self.resolve_local(&name) {
                    self.emit(Op::InitVar { depth, slot });
                } else {
                    self.emit(Op::Pop);
                }
            }
        }
        for stmt in &t.block.body {
            self.compile_statement(stmt)?;
        }
        self.emit(Op::PopEnv);
        self.pop_scope();
        let try_end = self.state.proto.body.len();
        if let Some(i) = catch_idx {
            self.state.proto.handlers[i as usize].end = try_end as u32;
        }
        if let Some(i) = finally_idx {
            self.state.proto.handlers[i as usize].end = try_end as u32;
        }

        let mut catch_range: Option<(usize, usize)> = None;
        if has_catch {
            self.state.current_handler = finally_idx;
            let skip = self.emit(Op::Jump(0));
            let catch_start = self.state.proto.body.len();
            if let Some(i) = catch_idx {
                self.state.proto.handlers[i as usize].catch_target = Some(catch_start as u32);
            }
            self.push_scope(ScopeKind::Catch);
            if let Some(h) = &t.handler {
                if let Some(p) = &h.param {
                    self.predeclare_binding_pattern(&p.pattern);
                }
                for stmt in &h.body.body {
                    self.predeclare_lexical(stmt, false);
                }
            }
            let catch_slots = self.scope().next_slot;
            self.emit(Op::CreateEnv(catch_slots as u32));
            if let Some(h) = &t.handler {
                if let Some(p) = &h.param {
                    self.bind_binding_pattern(&p.pattern, true)?;
                } else {
                    self.emit(Op::Pop);
                }
                for stmt in &h.body.body {
                    self.compile_statement(stmt)?;
                }
            }
            self.emit(Op::PopEnv);
            self.pop_scope();
            let catch_end = self.state.proto.body.len();
            catch_range = Some((catch_start, catch_end));
            self.patch_jump(skip, catch_end);
        }

        if has_finally {
            self.state.current_handler = outer_handler;
            let set_normal = self.emit(Op::SetNormalCompletion(0));
            let jump_finally = self.emit(Op::Jump(0));
            let finally_target = self.state.proto.body.len();
            if let Some(i) = finally_idx {
                self.state.proto.handlers[i as usize].finally_target = Some(finally_target as u32);
            }
            if let Some((cs, ce)) = catch_range {
                let cfi = self.state.proto.handlers.len() as u32;
                self.state.proto.handlers.push(Handler {
                    start: cs as u32,
                    end: ce as u32,
                    catch_target: None,
                    finally_target: Some(finally_target as u32),
                    env_depth,
                    stack_height: 0,
                    outer: outer_handler,
                });
                let _ = cfi;
            }
            self.compile_finally_block(t.finalizer.as_ref().unwrap(), finally_target)?;
            let after_finally = self.state.proto.body.len();
            if let Op::SetNormalCompletion(t) = &mut self.state.proto.body[set_normal] {
                *t = after_finally as u32;
            }
            self.patch_jump(jump_finally, finally_target);
            if let Some(i) = finally_idx {
                self.state.proto.handlers[i as usize].end =
                    catch_range.map(|(_, ce)| ce).unwrap_or(try_end) as u32;
            }
        }
        self.state.current_handler = outer_handler;
        Ok(())
    }
    fn compile_finally_block(&mut self, block: &BlockStatement, _target: usize) -> CResult<()> {
        self.push_scope(ScopeKind::Block);
        for stmt in &block.body {
            self.predeclare_lexical(stmt, false);
        }
        let slots = self.scope().next_slot;
        self.emit(Op::CreateEnv(slots as u32));
        for stmt in &block.body {
            self.compile_statement(stmt)?;
        }
        self.emit(Op::PopEnv);
        self.emit(Op::EndFinally);
        self.pop_scope();
        Ok(())
    }

    // ------------------------------------------------------------------
    // Assignment targets
    // ------------------------------------------------------------------
    fn bind_assignment_target(&mut self, target: &AssignmentTarget, _init: bool) -> CResult<()> {
        match target {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                if let Some((depth, slot, is_const)) = self.resolve_local(&name) {
                    if is_const {
                        self.emit(Op::ThrowConstAssignment);
                        return Ok(());
                    }
                    self.emit(Op::StoreVar { depth, slot });
                    self.emit(Op::Pop);
                } else {
                    let idx = self.emit_name_const(&name);
                    self.emit(Op::StoreGlobal {
                        name: idx,
                        strict: self.strict,
                    });
                    self.emit(Op::Pop);
                }
                Ok(())
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                let val = self.alloc_temp();
                self.emit(Op::StoreTemp(val));
                self.compile_expression(&m.object)?;
                self.compile_expression(&m.expression)?;
                self.emit(Op::LoadTemp(val));
                self.emit(Op::SetElem);
                self.emit(Op::Pop);
                Ok(())
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                let val = self.alloc_temp();
                self.emit(Op::StoreTemp(val));
                self.compile_expression(&m.object)?;
                self.emit(Op::LoadTemp(val));
                let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                self.emit(Op::SetProp(idx));
                self.emit(Op::Pop);
                Ok(())
            }

            AssignmentTarget::ArrayAssignmentTarget(a) => {
                let src = self.alloc_temp();
                let iter = self.alloc_temp();
                self.emit(Op::StoreTemp(src));
                self.emit(Op::LoadTemp(src));
                self.emit(Op::IterInit(iter));
                for e in &a.elements {
                    if let Some(e) = e {
                        let skip = self.emit(Op::IterNextDestructure {
                            temp: iter,
                            done: 0,
                        });
                        let cont = self.emit(Op::Jump(0));
                        let done_pos = self.state.proto.body.len();
                        self.patch_jump(skip, done_pos);
                        self.emit(Op::PushUndefined);
                        let after = self.state.proto.body.len();
                        self.patch_jump(cont, after);
                        self.bind_assignment_target_maybe_default(e)?;
                    } else {
                        let skip = self.emit(Op::IterNextDestructure {
                            temp: iter,
                            done: 0,
                        });
                        self.emit(Op::Pop);
                        let after = self.state.proto.body.len();
                        self.patch_jump(skip, after);
                    }
                }
                if let Some(rest) = &a.rest {
                    self.emit(Op::IterRest { temp: iter });
                    self.bind_assignment_target(&rest.target, false)?;
                }
                self.emit(Op::IterClose { temp: iter });
                Ok(())
            }
            AssignmentTarget::ObjectAssignmentTarget(o) => {
                let src = self.alloc_temp();
                self.emit(Op::StoreTemp(src));
                let mut excluded = Vec::new();
                for p in &o.properties {
                    match p {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(ip) => {
                            let name = JsString::from_str(ip.binding.name.as_str());
                            self.emit(Op::LoadTemp(src));
                            let idx = self.emit_name_const(&name);
                            self.emit(Op::GetProp(idx));
                            if let Some(init) = &ip.init {
                                let skip = self.emit(Op::JumpIfNotUndefined(0));
                                self.emit(Op::Pop);
                                self.compile_expression(init)?;
                                let after = self.state.proto.body.len();
                                self.patch_jump(skip, after);
                            }
                            excluded.push(name.clone());
                            self.bind_simple_identifier(&name)?;
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(pp) => {
                            self.emit(Op::LoadTemp(src));
                            let (computed, key_idx) = self.compile_property_key(&pp.name)?;
                            if computed {
                                self.emit(Op::GetElem);
                            } else {
                                self.emit(Op::GetProp(key_idx));
                            }
                            if let Some(n) = property_key_name(&pp.name) {
                                excluded.push(n);
                            }
                            self.bind_assignment_target_maybe_default(&pp.binding)?;
                        }
                    }
                }
                if let Some(rest) = &o.rest {
                    let count = excluded.len() as u32;
                    self.emit(Op::NewObject);
                    self.emit(Op::LoadTemp(src));
                    for name in &excluded {
                        let idx = self.emit_name_const(name);
                        self.emit(Op::PushConst(idx));
                    }
                    self.emit(Op::CopyDataPropertiesExcluding(count));
                    self.bind_assignment_target(&rest.target, false)?;
                }
                Ok(())
            }
            _ => Err(CompileError::new("unsupported assignment target")),
        }
    }

    fn bind_assignment_target_maybe_default(
        &mut self,
        target: &AssignmentTargetMaybeDefault,
    ) -> CResult<()> {
        match target {
            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(d) => {
                let skip = self.emit(Op::JumpIfNotUndefined(0));
                self.emit(Op::Pop);
                self.compile_expression(&d.init)?;
                let after = self.state.proto.body.len();
                self.patch_jump(skip, after);
                self.bind_assignment_target(&d.binding, false)
            }
            other => {
                if let Some(t) = other.as_assignment_target() {
                    self.bind_assignment_target(t, false)
                } else {
                    Err(CompileError::new("unsupported assignment target"))
                }
            }
        }
    }

    fn bind_simple_identifier(&mut self, name: &JsString) -> CResult<()> {
        if let Some((depth, slot, is_const)) = self.resolve_local(name) {
            if is_const {
                self.emit(Op::ThrowConstAssignment);
                return Ok(());
            }
            self.emit(Op::StoreVar { depth, slot });
            self.emit(Op::Pop);
        } else {
            let idx = self.emit_name_const(name);
            self.emit(Op::StoreGlobal {
                name: idx,
                strict: self.strict,
            });
            self.emit(Op::Pop);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------
    fn compile_expression(&mut self, expr: &Expression) -> CResult<()> {
        match expr {
            Expression::BooleanLiteral(b) => {
                self.emit(if b.value { Op::PushTrue } else { Op::PushFalse });
            }
            Expression::NullLiteral(_) => {
                self.emit(Op::PushNull);
            }
            Expression::NumericLiteral(n) => {
                if n.value == 0.0 && n.value.is_sign_negative() {
                    self.emit_push_const(Constant::Number(-0.0));
                } else {
                    self.emit_push_const(Constant::Number(n.value));
                }
            }
            Expression::BigIntLiteral(b) => {
                let value = parse_bigint_literal(b.value.as_str())
                    .ok_or_else(|| CompileError::new("invalid BigInt"))?;
                self.emit_push_const(Constant::BigInt(std::rc::Rc::new(value)));
            }
            Expression::StringLiteral(s) => {
                let js = jsstr_to_jstring(&s.value);
                self.emit_push_const(Constant::String(js));
            }
            Expression::RegExpLiteral(r) => {
                let pattern = reg_exp_pattern_text(&r.regex);
                let flags = regex_flags_to_string(&r.regex.flags);
                self.emit_push_const(Constant::RegExp {
                    source: JsString::from_str(&pattern),
                    flags: JsString::from_str(&flags),
                });
            }
            Expression::TemplateLiteral(t) => {
                self.compile_template_literal(t)?;
            }
            Expression::TaggedTemplateExpression(t) => {
                self.compile_tagged_template(t)?;
            }
            Expression::Identifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                if self.with_depth > 0 {
                    let local = self.resolve_local(&name);
                    let idx = self.emit_name_const(&name);
                    let (depth, slot, has_local) = match local {
                        Some((d, s, _)) => (d, s, true),
                        None => (0, 0, false),
                    };
                    self.emit(Op::LoadName {
                        name: idx,
                        depth,
                        slot,
                        has_local,
                    });
                } else if let Some((depth, slot, _)) = self.resolve_local(&name) {
                    self.emit(Op::LoadVar { depth, slot });
                } else {
                    let idx = self.emit_name_const(&name);
                    self.emit(Op::LoadGlobal(idx));
                }
            }
            Expression::StaticMemberExpression(m) => {
                if matches!(&m.object, Expression::Super(_)) {
                    let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                    self.emit(Op::GetSuperProp(idx));
                } else {
                    self.compile_expression(&m.object)?;
                    if m.optional {
                        self.emit(Op::Dup);
                        let jump = self.emit(Op::JumpIfNullishPop(0));
                        let idx =
                            self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                        self.emit(Op::GetProp(idx));
                        let end = self.state.proto.body.len();
                        self.patch_jump(jump, end);
                    } else {
                        let idx =
                            self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                        self.emit(Op::GetProp(idx));
                    }
                }
            }
            Expression::ComputedMemberExpression(m) => {
                if matches!(&m.object, Expression::Super(_)) {
                    self.compile_expression(&m.expression)?;
                    self.emit(Op::GetSuperElem);
                } else {
                    self.compile_expression(&m.object)?;
                    if m.optional {
                        self.emit(Op::Dup);
                        let jump = self.emit(Op::JumpIfNullishPop(0));
                        self.compile_expression(&m.expression)?;
                        self.emit(Op::GetElem);
                        let end = self.state.proto.body.len();
                        self.patch_jump(jump, end);
                    } else {
                        self.compile_expression(&m.expression)?;
                        self.emit(Op::GetElem);
                    }
                }
            }
            Expression::PrivateFieldExpression(m) => {
                let name = JsString::from_str(m.field.name.as_str());
                let sym = self
                    .private_names
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| new_private_symbol(&name));
                self.compile_expression(&m.object)?;
                self.emit_push_const(Constant::Symbol(sym));
                self.emit(Op::GetElem);
            }
            Expression::ThisExpression(_) => {
                self.emit(Op::LoadThis);
            }
            Expression::Super(_) => {
                return Err(CompileError::new("unexpected super expression"));
            }
            Expression::ArrayExpression(a) => {
                self.emit(Op::NewArray);
                let length_key = self.emit_name_const(&JsString::from_str("length"));
                for (i, elem) in a.elements.iter().enumerate() {
                    match elem {
                        ArrayExpressionElement::Elision(_) => {}
                        ArrayExpressionElement::SpreadElement(s) => {
                            // Ensure the array length is at least `i` before
                            // appending the spread values.
                            self.emit(Op::Dup);
                            let idx_const = self.emit_const(Constant::Number(i as f64));
                            self.emit(Op::PushConst(idx_const));
                            self.emit(Op::SetProp(length_key));
                            self.emit(Op::Pop);
                            self.emit(Op::Dup);
                            self.compile_expression(&s.argument)?;
                            self.emit(Op::ArrayPushSpread);
                            self.emit(Op::Pop);
                        }
                        other => {
                            if let Some(e) = other.as_expression() {
                                self.emit(Op::Dup);
                                let idx_const = self.emit_const(Constant::Number(i as f64));
                                self.emit(Op::PushConst(idx_const));
                                self.compile_expression(e)?;
                                self.emit(Op::SetElem);
                                self.emit(Op::Pop);
                            }
                        }
                    }
                }
                let len = a.elements.len() as u32;
                self.emit(Op::SetArrayLength(len));
            }
            Expression::ObjectExpression(o) => {
                self.emit(Op::NewObject);
                for prop in &o.properties {
                    match prop {
                        ObjectPropertyKind::SpreadProperty(s) => {
                            self.emit(Op::Dup);
                            self.compile_expression(&s.argument)?;
                            self.emit(Op::CopyDataProperties);
                            self.emit(Op::Pop);
                        }
                        ObjectPropertyKind::ObjectProperty(p) => {
                            let key_name = property_key_name(&p.key);
                            if p.kind == PropertyKind::Init
                                && !p.computed
                                && key_name
                                    .as_ref()
                                    .map(|n| n.to_string_lossy() == "__proto__")
                                    .unwrap_or(false)
                            {
                                self.compile_expression(&p.value)?;
                                self.emit(Op::SetProtoLiteral);
                                continue;
                            }
                            let (computed, key_idx) = self.compile_property_key(&p.key)?;
                            match p.kind {
                                PropertyKind::Init => {
                                    if p.method {
                                        self.emit(Op::Dup);
                                        self.compile_method_closure_with_home(&p.value)?;
                                    } else {
                                        self.compile_expression(&p.value)?;
                                    }
                                    self.emit(Op::DefineDataProp {
                                        key: key_idx,
                                        computed,
                                    });
                                }
                                PropertyKind::Get | PropertyKind::Set => {
                                    self.emit(Op::Dup);
                                    self.compile_method_closure_with_home(&p.value)?;
                                    self.emit(Op::DefineAccessorProp {
                                        key: key_idx,
                                        computed,
                                        getter: p.kind == PropertyKind::Get,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Expression::FunctionExpression(f) => {
                let name =
                    f.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                let proto =
                    self.compile_function_node(&**f, ProtoKind::Function, name, self.strict)?;
                let idx = self.add_nested_function(proto);
                self.emit(Op::CreateClosure(idx));
            }
            Expression::ArrowFunctionExpression(a) => {
                let proto = self.compile_arrow(a, JsString::empty(), self.strict)?;
                let idx = self.add_nested_function(proto);
                self.emit(Op::CreateClosure(idx));
            }
            Expression::ClassExpression(c) => {
                let name =
                    c.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                self.compile_class(c, name, false)?;
            }
            Expression::SequenceExpression(s) => {
                for (i, e) in s.expressions.iter().enumerate() {
                    self.compile_expression(e)?;
                    if i + 1 != s.expressions.len() {
                        self.emit(Op::Pop);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => {
                self.compile_expression(&p.expression)?;
            }
            Expression::ConditionalExpression(c) => {
                self.compile_expression(&c.test)?;
                let else_jump = self.emit(Op::JumpIfFalse(0));
                self.compile_expression(&c.consequent)?;
                let end_jump = self.emit(Op::Jump(0));
                let else_pos = self.state.proto.body.len();
                self.patch_jump(else_jump, else_pos);
                self.compile_expression(&c.alternate)?;
                let end = self.state.proto.body.len();
                self.patch_jump(end_jump, end);
            }
            Expression::LogicalExpression(l) => {
                self.compile_logical(l)?;
            }
            Expression::BinaryExpression(b) => {
                self.compile_expression(&b.left)?;
                self.compile_expression(&b.right)?;
                self.emit(Op::Binary(binary_op(b.operator)?));
            }
            Expression::UnaryExpression(u) => {
                if u.operator == UnaryOperator::Delete {
                    self.compile_delete(&u.argument)?;
                } else if u.operator == UnaryOperator::Typeof {
                    self.compile_typeof(&u.argument)?;
                } else {
                    self.compile_expression(&u.argument)?;
                    self.emit(Op::Unary(unary_op(u.operator)?));
                }
            }
            Expression::UpdateExpression(u) => {
                self.compile_update(u)?;
            }
            Expression::AssignmentExpression(a) => {
                self.compile_assignment(a)?;
            }
            Expression::CallExpression(c) => {
                self.compile_call(c)?;
            }
            Expression::NewExpression(n) => {
                self.compile_expression(&n.callee)?;
                if has_spread_args(&n.arguments) {
                    self.compile_arguments_array(&n.arguments)?;
                    self.emit(Op::NewSpread);
                } else {
                    for a in &n.arguments {
                        if let Some(e) = a.as_expression() {
                            self.compile_expression(e)?;
                        }
                    }
                    self.emit(Op::New(n.arguments.len() as u32));
                }
            }
            Expression::ChainExpression(c) => {
                self.compile_chain(&c.expression)?;
            }
            Expression::AwaitExpression(a) => {
                self.compile_expression(&a.argument)?;
                self.emit(Op::Await);
            }
            Expression::YieldExpression(y) => {
                let is_async_gen = self.state.proto.kind == ProtoKind::AsyncGenerator;
                if y.delegate {
                    if let Some(arg) = &y.argument {
                        self.compile_expression(arg)?;
                    } else {
                        self.emit(Op::PushUndefined);
                    }
                    let iter = self.alloc_temp();
                    if is_async_gen {
                        let res = self.alloc_temp();
                        self.emit(Op::AsyncIterInit(iter));
                        let loop_start = self.state.proto.body.len();
                        self.emit(Op::AsyncIterNext(iter));
                        self.emit(Op::Await);
                        self.emit(Op::StoreTemp(res));
                        self.emit(Op::LoadTemp(res));
                        let done_key = self.emit_name_const(&JsString::from_str("done"));
                        self.emit(Op::GetProp(done_key));
                        let done_jump = self.emit(Op::JumpIfTrue(0));
                        self.emit(Op::LoadTemp(res));
                        let value_key = self.emit_name_const(&JsString::from_str("value"));
                        self.emit(Op::GetProp(value_key));
                        self.emit(Op::Await);
                        self.emit(Op::Yield);
                        self.emit(Op::Pop);
                        self.emit(Op::Jump(loop_start as u32));
                        let done = self.state.proto.body.len();
                        self.patch_jump(done_jump, done);
                        self.emit(Op::PushUndefined);
                    } else {
                        self.emit(Op::IterInit(iter));
                        let loop_start = self.state.proto.body.len();
                        let done_jump = self.emit(Op::IterNext {
                            temp: iter,
                            done: 0,
                        });
                        self.emit(Op::Yield);
                        self.emit(Op::Pop);
                        self.emit(Op::Jump(loop_start as u32));
                        let done = self.state.proto.body.len();
                        self.patch_jump(done_jump, done);
                        self.emit(Op::IterClose { temp: iter });
                        self.emit(Op::PushUndefined);
                    }
                } else {
                    if let Some(arg) = &y.argument {
                        self.compile_expression(arg)?;
                    } else {
                        self.emit(Op::PushUndefined);
                    }
                    if is_async_gen {
                        self.emit(Op::Await);
                    }
                    self.emit(Op::Yield);
                }
            }
            Expression::NewTarget(_) => {
                self.emit(Op::LoadNewTarget);
            }
            Expression::ImportMeta(_) => {
                return Err(CompileError::new("import.meta is not supported"));
            }
            Expression::ImportExpression(_) => {
                return Err(CompileError::new("dynamic import is not supported"));
            }
            Expression::PrivateInExpression(p) => {
                let name = JsString::from_str(p.left.name.as_str());
                let sym = self
                    .private_names
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| new_private_symbol(&name));
                self.compile_expression(&p.right)?;
                self.emit_push_const(Constant::Symbol(sym));
                self.emit(Op::Swap);
                self.emit(Op::Binary(BinOp::In));
            }
            _ => {
                return Err(CompileError::new(format!(
                    "unsupported expression: {:?}",
                    std::mem::discriminant(expr)
                )))
            }
        }
        Ok(())
    }

    fn compile_method_function(&mut self, expr: &Expression) -> CResult<()> {
        match expr {
            Expression::FunctionExpression(f) => {
                let name =
                    f.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                let proto = self.compile_function_node(&**f, ProtoKind::Method, name, true)?;
                let idx = self.add_nested_function(proto);
                self.emit(Op::CreateClosure(idx));
                Ok(())
            }
            _ => Err(CompileError::new("expected method function")),
        }
    }

    fn compile_method_closure_with_home(&mut self, expr: &Expression) -> CResult<()> {
        match expr {
            Expression::FunctionExpression(f) => {
                let name =
                    f.id.as_ref()
                        .map(|i| JsString::from_str(i.name.as_str()))
                        .unwrap_or_default();
                let proto = self.compile_function_node(&**f, ProtoKind::Method, name, true)?;
                let idx = self.add_nested_function(proto);
                self.emit(Op::CreateClosureWithHome(idx));
                Ok(())
            }
            _ => Err(CompileError::new("expected method function")),
        }
    }

    fn compile_template_literal(&mut self, t: &TemplateLiteral) -> CResult<()> {
        let first = t
            .quasis
            .first()
            .and_then(|q| q.value.cooked.as_ref())
            .map(jsstr_to_jstring)
            .unwrap_or_default();
        self.emit_push_const(Constant::String(first));
        for (i, e) in t.expressions.iter().enumerate() {
            self.compile_expression(e)?;
            self.emit(Op::ToString);
            self.emit(Op::Binary(BinOp::Add));
            if let Some(q) = t.quasis.get(i + 1) {
                if let Some(c) = &q.value.cooked {
                    let s = jsstr_to_jstring(c);
                    self.emit_push_const(Constant::String(s));
                    self.emit(Op::Binary(BinOp::Add));
                }
            }
        }
        Ok(())
    }

    fn compile_tagged_template(&mut self, t: &TaggedTemplateExpression) -> CResult<()> {
        let cooked: Vec<Option<JsString>> = t
            .quasi
            .quasis
            .iter()
            .map(|q| q.value.cooked.as_ref().map(jsstr_to_jstring))
            .collect();
        let raw: Vec<JsString> = t
            .quasi
            .quasis
            .iter()
            .map(|q| JsString::from_str(q.value.raw.as_str()))
            .collect();
        let site_idx = self.emit_const(Constant::Template(TemplateSite { cooked, raw }));
        // Push tag and this.
        self.compile_callee_this(&t.tag)?;
        self.emit(Op::TemplateObject(site_idx));
        for e in &t.quasi.expressions {
            self.compile_expression(e)?;
        }
        self.emit(Op::Call(t.quasi.expressions.len() as u32 + 1));
        Ok(())
    }

    fn compile_callee_this(&mut self, callee: &Expression) -> CResult<()> {
        match callee {
            Expression::StaticMemberExpression(m) if matches!(&m.object, Expression::Super(_)) => {
                let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                self.emit(Op::GetSuperProp(idx));
                self.emit(Op::LoadThis);
            }
            Expression::ComputedMemberExpression(m)
                if matches!(&m.object, Expression::Super(_)) =>
            {
                self.compile_expression(&m.expression)?;
                self.emit(Op::GetSuperElem);
                self.emit(Op::LoadThis);
            }
            Expression::StaticMemberExpression(m) => {
                self.compile_expression(&m.object)?;
                self.emit(Op::Dup);
                let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                self.emit(Op::GetProp(idx));
                self.emit(Op::Swap);
            }
            Expression::ComputedMemberExpression(m) => {
                self.compile_expression(&m.object)?;
                self.emit(Op::Dup);
                self.compile_expression(&m.expression)?;
                self.emit(Op::GetElem);
                self.emit(Op::Swap);
            }
            _ => {
                self.compile_expression(callee)?;
                self.emit(Op::PushUndefined);
            }
        }
        Ok(())
    }
    fn compile_call(&mut self, call: &CallExpression) -> CResult<()> {
        // super(...) call
        if matches!(&call.callee, Expression::Super(_)) {
            if has_spread_args(&call.arguments) {
                self.compile_arguments_array(&call.arguments)?;
                self.emit(Op::SuperCallSpread);
            } else {
                for a in &call.arguments {
                    if let Some(e) = a.as_expression() {
                        self.compile_expression(e)?;
                    }
                }
                self.emit(Op::SuperCall(call.arguments.len() as u32));
            }
            return Ok(());
        }
        self.compile_callee_this(&call.callee)?;
        if has_spread_args(&call.arguments) {
            self.compile_arguments_array(&call.arguments)?;
            self.emit(Op::CallSpread);
        } else {
            for a in &call.arguments {
                if let Some(e) = a.as_expression() {
                    self.compile_expression(e)?;
                }
            }
            self.emit(Op::Call(call.arguments.len() as u32));
        }
        Ok(())
    }

    fn compile_arguments_array(&mut self, args: &[Argument]) -> CResult<()> {
        self.emit(Op::NewArray);
        for a in args {
            match a {
                Argument::SpreadElement(s) => {
                    self.emit(Op::Dup);
                    self.compile_expression(&s.argument)?;
                    self.emit(Op::ArrayPushSpread);
                    self.emit(Op::Pop);
                }
                other => {
                    if let Some(e) = other.as_expression() {
                        self.emit(Op::Dup);
                        self.compile_expression(e)?;
                        self.emit(Op::ArrayPush);
                        self.emit(Op::Pop);
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_logical(&mut self, l: &LogicalExpression) -> CResult<()> {
        self.compile_expression(&l.left)?;
        match l.operator {
            LogicalOperator::And => {
                self.emit(Op::Dup);
                let jump = self.emit(Op::JumpIfFalsePop(0));
                self.emit(Op::Pop);
                self.compile_expression(&l.right)?;
                let end = self.state.proto.body.len();
                self.patch_jump(jump, end);
            }
            LogicalOperator::Or => {
                self.emit(Op::Dup);
                let jump = self.emit(Op::JumpIfTruePop(0));
                self.emit(Op::Pop);
                self.compile_expression(&l.right)?;
                let end = self.state.proto.body.len();
                self.patch_jump(jump, end);
            }
            LogicalOperator::Coalesce => {
                self.emit(Op::Dup);
                let jump = self.emit(Op::JumpIfNotNullishPop(0));
                self.emit(Op::Pop);
                self.compile_expression(&l.right)?;
                let end = self.state.proto.body.len();
                self.patch_jump(jump, end);
            }
        }
        Ok(())
    }

    fn compile_delete(&mut self, expr: &Expression) -> CResult<()> {
        match expr {
            Expression::Identifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                if self.resolve_local(&name).is_some() {
                    self.emit(Op::PushFalse);
                } else {
                    let idx = self.emit_name_const(&name);
                    self.emit(Op::DeleteGlobal(idx));
                }
            }
            Expression::StaticMemberExpression(m) => {
                self.compile_expression(&m.object)?;
                let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                self.emit(Op::DeleteProp(idx));
            }
            Expression::ComputedMemberExpression(m) => {
                self.compile_expression(&m.object)?;
                self.compile_expression(&m.expression)?;
                self.emit(Op::DeleteElem);
            }
            _ => {
                self.compile_expression(expr)?;
                self.emit(Op::Pop);
                self.emit(Op::PushTrue);
            }
        }
        Ok(())
    }

    fn compile_typeof(&mut self, expr: &Expression) -> CResult<()> {
        if let Expression::Identifier(id) = expr {
            let name = JsString::from_str(id.name.as_str());
            if let Some((depth, slot, _)) = self.resolve_local(&name) {
                self.emit(Op::LoadVar { depth, slot });
                self.emit(Op::Unary(UnaryOp::TypeOf));
            } else {
                let idx = self.emit_name_const(&name);
                self.emit(Op::TypeOfGlobal(idx));
            }
            return Ok(());
        }
        self.compile_expression(expr)?;
        self.emit(Op::Unary(UnaryOp::TypeOf));
        Ok(())
    }

    fn compile_update(&mut self, u: &UpdateExpression) -> CResult<()> {
        let op = if u.operator == UpdateOperator::Increment {
            UnaryOp::Increment
        } else {
            UnaryOp::Decrement
        };
        match &u.argument {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                let local = self.resolve_local(&name);
                if let Some((depth, slot, is_const)) = local {
                    if is_const {
                        return Err(CompileError::new("assignment to constant"));
                    }
                    self.emit(Op::LoadVar { depth, slot });
                } else {
                    let idx = self.emit_name_const(&name);
                    self.emit(Op::LoadGlobal(idx));
                }
                let old = self.alloc_temp();
                let new = self.alloc_temp();
                self.emit(Op::StoreTemp(old));
                self.emit(Op::LoadTemp(old));
                self.emit(Op::Unary(op));
                self.emit(Op::StoreTemp(new));
                if let Some((depth, slot, _)) = local {
                    self.emit(Op::LoadTemp(new));
                    self.emit(Op::StoreVar { depth, slot });
                    self.emit(Op::Pop);
                } else {
                    let idx = self.emit_name_const(&name);
                    self.emit(Op::LoadTemp(new));
                    self.emit(Op::StoreGlobal {
                        name: idx,
                        strict: self.strict,
                    });
                    self.emit(Op::Pop);
                }
                if u.prefix {
                    self.emit(Op::LoadTemp(new));
                } else {
                    self.emit(Op::LoadTemp(old));
                }
            }
            SimpleAssignmentTarget::StaticMemberExpression(m) => {
                let obj = self.alloc_temp();
                let old = self.alloc_temp();
                let new = self.alloc_temp();
                self.compile_expression(&m.object)?;
                self.emit(Op::StoreTemp(obj));
                self.emit(Op::LoadTemp(obj));
                let key_idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                self.emit(Op::GetProp(key_idx));
                self.emit(Op::StoreTemp(old));
                self.emit(Op::LoadTemp(old));
                self.emit(Op::Unary(op));
                self.emit(Op::StoreTemp(new));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(new));
                self.emit(Op::SetProp(key_idx));
                self.emit(Op::Pop);
                if u.prefix {
                    self.emit(Op::LoadTemp(new));
                } else {
                    self.emit(Op::LoadTemp(old));
                }
            }
            SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                let obj = self.alloc_temp();
                let key = self.alloc_temp();
                let old = self.alloc_temp();
                let new = self.alloc_temp();
                self.compile_expression(&m.object)?;
                self.emit(Op::StoreTemp(obj));
                self.compile_expression(&m.expression)?;
                self.emit(Op::StoreTemp(key));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(key));
                self.emit(Op::GetElem);
                self.emit(Op::StoreTemp(old));
                self.emit(Op::LoadTemp(old));
                self.emit(Op::Unary(op));
                self.emit(Op::StoreTemp(new));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(key));
                self.emit(Op::LoadTemp(new));
                self.emit(Op::SetElem);
                self.emit(Op::Pop);
                if u.prefix {
                    self.emit(Op::LoadTemp(new));
                } else {
                    self.emit(Op::LoadTemp(old));
                }
            }
            SimpleAssignmentTarget::PrivateFieldExpression(m) => {
                let name = JsString::from_str(m.field.name.as_str());
                let sym = self
                    .private_names
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| new_private_symbol(&name));
                self.compile_expression(&m.object)?;
                let obj = self.alloc_temp();
                self.emit(Op::StoreTemp(obj));
                let key = self.alloc_temp();
                self.emit_push_const(Constant::Symbol(sym));
                self.emit(Op::StoreTemp(key));
                let old = self.alloc_temp();
                let new = self.alloc_temp();
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(key));
                self.emit(Op::GetElem);
                self.emit(Op::StoreTemp(old));
                self.emit(Op::LoadTemp(old));
                self.emit(Op::Unary(op));
                self.emit(Op::StoreTemp(new));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(key));
                self.emit(Op::LoadTemp(new));
                self.emit(Op::SetElem);
                self.emit(Op::Pop);
                if u.prefix {
                    self.emit(Op::LoadTemp(new));
                } else {
                    self.emit(Op::LoadTemp(old));
                }
            }
            _ => return Err(CompileError::new("unsupported update target")),
        }
        Ok(())
    }
    fn compile_assignment(&mut self, a: &AssignmentExpression) -> CResult<()> {
        if a.operator == AssignmentOperator::Assign {
            match &a.left {
                AssignmentTarget::AssignmentTargetIdentifier(id) => {
                    self.compile_expression(&a.right)?;
                    let name = JsString::from_str(id.name.as_str());
                    if let Some((depth, slot, is_const)) = self.resolve_local(&name) {
                        if is_const {
                            self.emit(Op::ThrowConstAssignment);
                        } else {
                            self.emit(Op::StoreVar { depth, slot });
                        }
                    } else {
                        let idx = self.emit_name_const(&name);
                        self.emit(Op::StoreGlobal {
                            name: idx,
                            strict: self.strict,
                        });
                    }
                    return Ok(());
                }
                AssignmentTarget::PrivateFieldExpression(m) => {
                    let name = JsString::from_str(m.field.name.as_str());
                    let sym = self
                        .private_names
                        .get(&name)
                        .cloned()
                        .unwrap_or_else(|| new_private_symbol(&name));
                    self.compile_expression(&m.object)?;
                    self.emit_push_const(Constant::Symbol(sym));
                    self.compile_expression(&a.right)?;
                    self.emit(Op::SetElem);
                    return Ok(());
                }
                AssignmentTarget::StaticMemberExpression(m) => {
                    self.compile_expression(&m.object)?;
                    self.compile_expression(&a.right)?;
                    let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                    self.emit(Op::SetProp(idx));
                    return Ok(());
                }
                AssignmentTarget::ComputedMemberExpression(m) => {
                    self.compile_expression(&m.object)?;
                    self.compile_expression(&m.expression)?;
                    self.compile_expression(&a.right)?;
                    self.emit(Op::SetElem);
                    return Ok(());
                }
                _ => {
                    self.compile_expression(&a.right)?;
                    let val = self.alloc_temp();
                    self.emit(Op::StoreTemp(val));
                    self.bind_assignment_target(&a.left, false)?;
                    self.emit(Op::LoadTemp(val));
                    return Ok(());
                }
            }
        }
        // Compound assignment.
        let bin = match a.operator {
            AssignmentOperator::Addition => BinOp::Add,
            AssignmentOperator::Subtraction => BinOp::Sub,
            AssignmentOperator::Multiplication => BinOp::Mul,
            AssignmentOperator::Division => BinOp::Div,
            AssignmentOperator::Remainder => BinOp::Mod,
            AssignmentOperator::Exponential => BinOp::Pow,
            AssignmentOperator::ShiftLeft => BinOp::Shl,
            AssignmentOperator::ShiftRight => BinOp::Shr,
            AssignmentOperator::ShiftRightZeroFill => BinOp::UShr,
            AssignmentOperator::BitwiseOR => BinOp::BitOr,
            AssignmentOperator::BitwiseXOR => BinOp::BitXor,
            AssignmentOperator::BitwiseAnd => BinOp::BitAnd,
            AssignmentOperator::LogicalOr
            | AssignmentOperator::LogicalAnd
            | AssignmentOperator::LogicalNullish => {
                return self.compile_logical_assignment(a);
            }
            AssignmentOperator::Assign => unreachable!(),
        };
        match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                if let Some((depth, slot, is_const)) = self.resolve_local(&name) {
                    if is_const {
                        self.emit(Op::ThrowConstAssignment);
                        return Ok(());
                    }
                    self.emit(Op::LoadVar { depth, slot });
                    self.compile_expression(&a.right)?;
                    self.emit(Op::Binary(bin));
                    self.emit(Op::StoreVar { depth, slot });
                } else {
                    let idx = self.emit_name_const(&name);
                    self.emit(Op::LoadGlobal(idx));
                    self.compile_expression(&a.right)?;
                    self.emit(Op::Binary(bin));
                    self.emit(Op::StoreGlobal {
                        name: idx,
                        strict: self.strict,
                    });
                }
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                let obj = self.alloc_temp();
                let val = self.alloc_temp();
                self.compile_expression(&m.object)?;
                self.emit(Op::StoreTemp(obj));
                self.emit(Op::LoadTemp(obj));
                let key_idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                self.emit(Op::GetProp(key_idx));
                self.compile_expression(&a.right)?;
                self.emit(Op::Binary(bin));
                self.emit(Op::StoreTemp(val));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(val));
                self.emit(Op::SetProp(key_idx));
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                let obj = self.alloc_temp();
                let key = self.alloc_temp();
                let val = self.alloc_temp();
                self.compile_expression(&m.object)?;
                self.emit(Op::StoreTemp(obj));
                self.compile_expression(&m.expression)?;
                self.emit(Op::StoreTemp(key));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(key));
                self.emit(Op::GetElem);
                self.compile_expression(&a.right)?;
                self.emit(Op::Binary(bin));
                self.emit(Op::StoreTemp(val));
                self.emit(Op::LoadTemp(obj));
                self.emit(Op::LoadTemp(key));
                self.emit(Op::LoadTemp(val));
                self.emit(Op::SetElem);
            }
            _ => return Err(CompileError::new("unsupported compound assignment target")),
        }
        Ok(())
    }

    fn compile_logical_assignment(&mut self, a: &AssignmentExpression) -> CResult<()> {
        // Only identifier targets are handled here.
        match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                let name = JsString::from_str(id.name.as_str());
                let (depth, slot, is_const) = match self.resolve_local(&name) {
                    Some(v) => v,
                    None => {
                        return Err(CompileError::new(
                            "logical assignment to global is not supported",
                        ))
                    }
                };
                if is_const {
                    self.emit(Op::ThrowConstAssignment);
                    return Ok(());
                }
                self.emit(Op::LoadVar { depth, slot });
                self.emit(Op::Dup);
                let jump = match a.operator {
                    AssignmentOperator::LogicalAnd => self.emit(Op::JumpIfFalsePop(0)),
                    AssignmentOperator::LogicalOr => self.emit(Op::JumpIfTruePop(0)),
                    _ => self.emit(Op::JumpIfNotNullishPop(0)),
                };
                self.emit(Op::Pop);
                self.compile_expression(&a.right)?;
                self.emit(Op::StoreVar { depth, slot });
                let end = self.state.proto.body.len();
                self.patch_jump(jump, end);
                Ok(())
            }
            _ => Err(CompileError::new("unsupported logical assignment target")),
        }
    }

    fn compile_chain(&mut self, element: &ChainElement) -> CResult<()> {
        match element {
            ChainElement::CallExpression(call) => {
                if call.optional {
                    self.compile_callee_this(&call.callee)?;
                    self.emit(Op::Dup);
                    let jump = self.emit(Op::JumpIfNullishPop(0));
                    // stack now has callee, this; pop and yield undefined
                    self.emit(Op::Pop);
                    self.emit(Op::Pop);
                    self.emit(Op::PushUndefined);
                    let end_jump = self.emit(Op::Jump(0));
                    let after = self.state.proto.body.len();
                    self.patch_jump(jump, after);
                    // stack has callee, this (non-nullish); call
                    if has_spread_args(&call.arguments) {
                        self.compile_arguments_array(&call.arguments)?;
                        self.emit(Op::CallSpread);
                    } else {
                        for a in &call.arguments {
                            if let Some(e) = a.as_expression() {
                                self.compile_expression(e)?;
                            }
                        }
                        self.emit(Op::Call(call.arguments.len() as u32));
                    }
                    let end = self.state.proto.body.len();
                    self.patch_jump(end_jump, end);
                    return Ok(());
                }
                self.compile_call(call)
            }
            ChainElement::StaticMemberExpression(m) => {
                self.compile_expression(&m.object)?;
                if m.optional {
                    self.emit(Op::Dup);
                    let jump = self.emit(Op::JumpIfNullishPop(0));
                    let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                    self.emit(Op::GetProp(idx));
                    let end = self.state.proto.body.len();
                    self.patch_jump(jump, end);
                } else {
                    let idx = self.emit_name_const(&JsString::from_str(m.property.name.as_str()));
                    self.emit(Op::GetProp(idx));
                }
                Ok(())
            }
            ChainElement::ComputedMemberExpression(m) => {
                self.compile_expression(&m.object)?;
                if m.optional {
                    self.emit(Op::Dup);
                    let jump = self.emit(Op::JumpIfNullishPop(0));
                    self.compile_expression(&m.expression)?;
                    self.emit(Op::GetElem);
                    let end = self.state.proto.body.len();
                    self.patch_jump(jump, end);
                } else {
                    self.compile_expression(&m.expression)?;
                    self.emit(Op::GetElem);
                }
                Ok(())
            }
            ChainElement::TSNonNullExpression(_) => {
                Err(CompileError::new("TS syntax not supported"))
            }
            ChainElement::PrivateFieldExpression(m) => {
                let name = JsString::from_str(m.field.name.as_str());
                let sym = self
                    .private_names
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| new_private_symbol(&name));
                self.compile_expression(&m.object)?;
                self.emit_push_const(Constant::Symbol(sym));
                self.emit(Op::GetElem);
                Ok(())
            }
        }
    }

    fn compile_class(&mut self, class: &Class, name: JsString, _decl: bool) -> CResult<()> {
        {
            let mut private_seen: HashMap<String, u8> = HashMap::new();
            for element in &class.body.body {
                let (key, computed, is_static, accessor) = match element {
                    ClassElement::MethodDefinition(m) => (
                        Some(&m.key),
                        m.computed,
                        m.r#static,
                        m.kind != MethodDefinitionKind::Method,
                    ),
                    ClassElement::PropertyDefinition(p) => {
                        (Some(&p.key), p.computed, p.r#static, false)
                    }
                    _ => (None, false, false, false),
                };
                if let Some(key) = key {
                    let name = match key {
                        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str().to_string()),
                        PropertyKey::PrivateIdentifier(id) => Some(id.name.as_str().to_string()),
                        _ => None,
                    };
                    if let Some(n) = &name {
                        if matches!(key, PropertyKey::PrivateIdentifier(_)) {
                            let kind = match element {
                                ClassElement::MethodDefinition(m) => match m.kind {
                                    MethodDefinitionKind::Get => 1,
                                    MethodDefinitionKind::Set => 2,
                                    _ => 0,
                                },
                                _ => 0,
                            };
                            let seen = private_seen.entry(n.clone()).or_insert(0);
                            if kind != 0 {
                                if (*seen & kind) != 0 {
                                    return Err(CompileError::new("duplicate private name"));
                                }
                                *seen |= kind;
                            } else {
                                if *seen != 0 {
                                    return Err(CompileError::new("duplicate private name"));
                                }
                                *seen = 4;
                            }
                        }
                        if n == "constructor"
                            && !matches!(element, ClassElement::MethodDefinition(_))
                        {
                            return Err(CompileError::new(
                                "classes may not have a field named constructor",
                            ));
                        }
                        if is_static && n == "prototype" && !computed {
                            return Err(CompileError::new(
                                "classes may not have a static property named prototype",
                            ));
                        }
                    }
                }
                let mut exprs: Vec<&Expression> = Vec::new();
                if let ClassElement::PropertyDefinition(p) = element {
                    if let Some(v) = &p.value {
                        exprs.push(v);
                    }
                }
                if computed {
                    if let Some(e) = key.and_then(|k| k.as_expression()) {
                        exprs.push(e);
                    }
                }
                for e in exprs {
                    let sp = e.span();
                    let text =
                        &self.source[sp.start as usize..(sp.end as usize).min(self.source.len())];
                    if contains_word(text, "arguments") {
                        return Err(CompileError::new(
                            "arguments is not allowed in a class field initializer",
                        ));
                    }
                    if text.contains("super(") || text.contains("super (") {
                        return Err(CompileError::new(
                            "super() is not allowed in a class field initializer",
                        ));
                    }
                }
            }
        }
        let saved_private = self.private_names.clone();
        for element in &class.body.body {
            let key = match element {
                ClassElement::MethodDefinition(m) => Some(&m.key),
                ClassElement::PropertyDefinition(p) => Some(&p.key),
                _ => None,
            };
            if let Some(PropertyKey::PrivateIdentifier(id)) = key {
                let n = JsString::from_str(id.name.as_str());
                if !self.private_names.contains_key(&n) {
                    self.private_names.insert(n.clone(), new_private_symbol(&n));
                }
            }
        }
        let derived = class.heritage.is_some();
        let h = self.alloc_temp();
        let p = self.alloc_temp();
        let c = self.alloc_temp();
        if let Some(heritage) = &class.heritage {
            self.compile_expression(&heritage.expression)?;
        } else {
            self.emit(Op::PushUndefined);
        }
        self.emit(Op::StoreTemp(h));
        if derived {
            self.emit(Op::LoadTemp(h));
            let null_jump = self.emit(Op::JumpIfNullishPop(0));
            self.emit(Op::LoadTemp(h));
            let key = self.emit_name_const(&JsString::from_str("prototype"));
            self.emit(Op::GetProp(key));
            let done = self.emit(Op::Jump(0));
            let null_label = self.state.proto.body.len();
            self.patch_jump(null_jump, null_label);
            self.emit(Op::PushNull);
            let after = self.state.proto.body.len();
            self.patch_jump(done, after);
        } else {
            let obj_name = self.emit_name_const(&JsString::from_str("Object"));
            self.emit(Op::LoadGlobal(obj_name));
            let key = self.emit_name_const(&JsString::from_str("prototype"));
            self.emit(Op::GetProp(key));
        }
        self.emit(Op::NewObject);
        self.emit(Op::Swap);
        self.emit(Op::SetProtoLiteral);
        self.emit(Op::StoreTemp(p));

        let mut ctor_method: Option<&MethodDefinition> = None;
        for element in &class.body.body {
            if let ClassElement::MethodDefinition(m) = element {
                if !m.r#static && m.kind == MethodDefinitionKind::Constructor {
                    ctor_method = Some(m);
                    break;
                }
            }
        }
        let ctor_proto = match ctor_method {
            Some(m) => {
                let kind = if derived {
                    ProtoKind::DerivedConstructor
                } else {
                    ProtoKind::ClassConstructor
                };
                self.compile_function_node(&m.value, kind, name.clone(), true)?
            }
            None => {
                let kind = if derived {
                    ProtoKind::DerivedConstructor
                } else {
                    ProtoKind::ClassConstructor
                };
                let mut proto = FunctionProto::new(name.clone(), kind);
                proto.is_strict = true;
                if derived {
                    proto.body.push(Op::PushArgsArray);
                    proto.body.push(Op::SuperCallSpread);
                    proto.body.push(Op::ReturnUndefined);
                } else {
                    proto.body.push(Op::ReturnUndefined);
                }
                Arc::new(proto)
            }
        };
        let ctor_idx = self.add_nested_function(ctor_proto);
        self.emit(Op::LoadTemp(p));
        self.emit(Op::CreateClosureWithHome(ctor_idx));
        self.emit(Op::StoreTemp(c));
        if derived {
            self.emit(Op::LoadTemp(c));
            self.emit(Op::LoadTemp(h));
            self.emit(Op::SetPrototypeOf);
            self.emit(Op::Pop);
        }
        self.emit(Op::LoadTemp(c));
        self.emit(Op::LoadTemp(p));
        self.emit(Op::DefineClassPrototype);
        self.emit(Op::Pop);
        self.emit(Op::LoadTemp(p));
        self.emit(Op::LoadTemp(c));
        self.emit(Op::DefineClassConstructor);
        self.emit(Op::Pop);

        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(m) => {
                    if !m.r#static && m.kind == MethodDefinitionKind::Constructor {
                        continue;
                    }
                    let target = if m.r#static { c } else { p };
                    self.emit(Op::LoadTemp(target));
                    self.push_class_key(&m.key, m.computed)?;
                    self.emit(Op::LoadTemp(target));
                    let (kind, method_kind) = match m.kind {
                        MethodDefinitionKind::Get => (ProtoKind::Getter, 1u8),
                        MethodDefinitionKind::Set => (ProtoKind::Setter, 2u8),
                        _ => (ProtoKind::Method, 0u8),
                    };
                    let proto = self.compile_function_node(&m.value, kind, name.clone(), true)?;
                    let idx = self.add_nested_function(proto);
                    self.emit(Op::CreateClosureWithHome(idx));
                    self.emit(Op::DefineClassMethod { kind: method_kind });
                    self.emit(Op::Pop);
                }
                ClassElement::PropertyDefinition(field) => {
                    self.emit(Op::LoadTemp(c));
                    self.push_class_key(&field.key, field.computed)?;
                    let home = if field.r#static { c } else { p };
                    self.emit(Op::LoadTemp(home));
                    if let Some(value) = &field.value {
                        let proto = self.compile_field_initializer(Some(value), name.clone())?;
                        let idx = self.add_nested_function(proto);
                        self.emit(Op::CreateClosureWithHome(idx));
                    } else {
                        self.emit(Op::PushUndefined);
                    }
                    self.emit(Op::AddClassField {
                        is_static: field.r#static,
                    });
                    self.emit(Op::Pop);
                }
                ClassElement::StaticBlock(block) => {
                    self.emit(Op::LoadTemp(c));
                    self.emit(Op::Dup);
                    let proto = self.compile_static_block(block, name.clone())?;
                    let idx = self.add_nested_function(proto);
                    self.emit(Op::CreateClosureWithHome(idx));
                    self.emit(Op::Swap);
                    self.emit(Op::Call(0));
                    self.emit(Op::Pop);
                    self.emit(Op::Pop);
                }
                _ => return Err(CompileError::new("unsupported class element")),
            }
        }
        self.emit(Op::LoadTemp(c));
        self.private_names = saved_private;
        Ok(())
    }

    fn push_class_key(&mut self, key: &PropertyKey, computed: bool) -> CResult<()> {
        if computed {
            if let Some(e) = key.as_expression() {
                self.compile_expression(e)?;
            } else {
                self.emit(Op::PushUndefined);
            }
        } else if let PropertyKey::PrivateIdentifier(id) = key {
            let n = JsString::from_str(id.name.as_str());
            let sym = self
                .private_names
                .get(&n)
                .cloned()
                .unwrap_or_else(|| new_private_symbol(&n));
            self.emit_push_const(Constant::Symbol(sym));
        } else if let Some(name) = property_key_name(key) {
            let idx = self.emit_name_const(&name);
            self.emit(Op::PushConst(idx));
        } else {
            return Err(CompileError::new("unsupported class property key"));
        }
        Ok(())
    }

    fn compile_field_initializer(
        &mut self,
        expr: Option<&Expression>,
        name: JsString,
    ) -> CResult<Arc<FunctionProto>> {
        let outer = self.capture_outer();
        let mut c = Compiler::new(self.source, true, ProtoKind::Method);
        c.outer_scopes = outer;
        c.private_names = self.private_names.clone();
        c.setup_scope(ScopeKind::Function);
        c.state.proto.name = name;
        c.state.proto.is_strict = true;
        c.state.proto.env_slots = 0;
        match expr {
            Some(e) => {
                c.compile_expression(e)?;
                c.emit(Op::Return);
            }
            None => {
                c.emit(Op::ReturnUndefined);
            }
        }
        c.state.proto.temp_slots = c.state.temps;
        Ok(Arc::new(c.finish()))
    }

    fn compile_static_block(
        &mut self,
        block: &StaticBlock,
        name: JsString,
    ) -> CResult<Arc<FunctionProto>> {
        let outer = self.capture_outer();
        let mut c = Compiler::new(self.source, true, ProtoKind::StaticBlock);
        c.outer_scopes = outer;
        c.private_names = self.private_names.clone();
        c.setup_scope(ScopeKind::Function);
        c.state.proto.name = name;
        c.state.proto.is_strict = true;
        let mut var_names = Vec::new();
        for stmt in &block.body {
            collect_var_names(stmt, &mut var_names);
        }
        for n in var_names {
            c.declare(n, false);
        }
        for stmt in &block.body {
            c.predeclare_lexical(stmt, true);
        }
        c.state.proto.env_slots = c.scope().next_slot as u32;
        for stmt in &block.body {
            c.compile_statement(stmt)?;
        }
        c.emit(Op::ReturnUndefined);
        c.state.proto.temp_slots = c.state.temps;
        Ok(Arc::new(c.finish()))
    }
}

fn property_key_name(key: &PropertyKey) -> Option<JsString> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(JsString::from_str(id.name.as_str())),
        PropertyKey::StringLiteral(s) => Some(jsstr_to_jstring(&s.value)),
        PropertyKey::NumericLiteral(n) => {
            Some(JsString::from_str(&crate::value::format_number(n.value)))
        }
        _ => None,
    }
}

fn jsstr_to_jstring(s: &oxc_str::JSStr<'_>) -> JsString {
    JsString::from_utf16(s.encode_utf16().collect())
}

fn reg_exp_pattern_text(regex: &RegExp<'_>) -> String {
    regex.pattern.text.to_string()
}

fn regex_flags_to_string(flags: &RegExpFlags) -> String {
    let mut out = String::new();
    let names = [
        ("d", RegExpFlags::D),
        ("g", RegExpFlags::G),
        ("i", RegExpFlags::I),
        ("m", RegExpFlags::M),
        ("s", RegExpFlags::S),
        ("u", RegExpFlags::U),
        ("v", RegExpFlags::V),
        ("y", RegExpFlags::Y),
    ];
    for (n, f) in names {
        if flags.contains(f) {
            out.push_str(n);
        }
    }
    out
}

fn parse_bigint_literal(s: &str) -> Option<BigInt> {
    let t = s.trim_end_matches('n');
    let (radix, digits) = if let Some(r) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, r)
    } else if let Some(r) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        (8, r)
    } else if let Some(r) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (2, r)
    } else {
        (10, t)
    };
    BigInt::parse_bytes(digits.replace('_', "").as_bytes(), radix)
}

fn binary_op(op: BinaryOperator) -> CResult<BinOp> {
    Ok(match op {
        BinaryOperator::Equality => BinOp::Eq,
        BinaryOperator::Inequality => BinOp::Ne,
        BinaryOperator::StrictEquality => BinOp::StrictEq,
        BinaryOperator::StrictInequality => BinOp::StrictNe,
        BinaryOperator::LessThan => BinOp::Lt,
        BinaryOperator::LessEqualThan => BinOp::Le,
        BinaryOperator::GreaterThan => BinOp::Gt,
        BinaryOperator::GreaterEqualThan => BinOp::Ge,
        BinaryOperator::Addition => BinOp::Add,
        BinaryOperator::Subtraction => BinOp::Sub,
        BinaryOperator::Multiplication => BinOp::Mul,
        BinaryOperator::Division => BinOp::Div,
        BinaryOperator::Remainder => BinOp::Mod,
        BinaryOperator::Exponential => BinOp::Pow,
        BinaryOperator::ShiftLeft => BinOp::Shl,
        BinaryOperator::ShiftRight => BinOp::Shr,
        BinaryOperator::ShiftRightZeroFill => BinOp::UShr,
        BinaryOperator::BitwiseOR => BinOp::BitOr,
        BinaryOperator::BitwiseXOR => BinOp::BitXor,
        BinaryOperator::BitwiseAnd => BinOp::BitAnd,
        BinaryOperator::In => BinOp::In,
        BinaryOperator::Instanceof => BinOp::InstanceOf,
    })
}

fn unary_op(op: UnaryOperator) -> CResult<UnaryOp> {
    Ok(match op {
        UnaryOperator::UnaryPlus => UnaryOp::Plus,
        UnaryOperator::UnaryNegation => UnaryOp::Minus,
        UnaryOperator::LogicalNot => UnaryOp::Not,
        UnaryOperator::BitwiseNot => UnaryOp::BitNot,
        UnaryOperator::Typeof => UnaryOp::TypeOf,
        UnaryOperator::Void => UnaryOp::Void,
        UnaryOperator::Delete => UnaryOp::Delete,
    })
}

fn has_spread_args(args: &[Argument]) -> bool {
    args.iter().any(|a| matches!(a, Argument::SpreadElement(_)))
}

fn collect_pattern_names(pattern: &BindingPattern, out: &mut Vec<JsString>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.push(JsString::from_str(id.name.as_str())),
        BindingPattern::ObjectPattern(o) => {
            for p in &o.properties {
                collect_pattern_names(&p.value, out);
            }
            if let Some(r) = &o.rest {
                collect_pattern_names(&r.argument, out);
            }
        }
        BindingPattern::ArrayPattern(a) => {
            for e in &a.elements {
                if let Some(e) = e {
                    collect_pattern_names(e, out);
                }
            }
            if let Some(r) = &a.rest {
                collect_pattern_names(&r.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(a) => collect_pattern_names(&a.left, out),
    }
}

fn collect_var_names(stmt: &Statement, out: &mut Vec<JsString>) {
    match stmt {
        Statement::VariableDeclaration(v) => {
            if v.kind == VariableDeclarationKind::Var {
                for d in &v.declarations {
                    collect_pattern_names(&d.id, out);
                }
            }
        }
        Statement::BlockStatement(b) => {
            for s in &b.body {
                collect_var_names(s, out);
            }
        }
        Statement::IfStatement(i) => {
            collect_var_names(&i.consequent, out);
            if let Some(a) = &i.alternate {
                collect_var_names(a, out);
            }
        }
        Statement::WhileStatement(w) => collect_var_names(&w.body, out),
        Statement::DoWhileStatement(d) => collect_var_names(&d.body, out),
        Statement::ForStatement(f) => {
            if let Some(ForStatementInit::VariableDeclaration(v)) = &f.init {
                if v.kind == VariableDeclarationKind::Var {
                    for d in &v.declarations {
                        collect_pattern_names(&d.id, out);
                    }
                }
            }
            collect_var_names(&f.body, out);
        }
        Statement::ForInStatement(f) => {
            if let ForStatementLeft::VariableDeclaration(v) = &f.left {
                if v.kind == VariableDeclarationKind::Var {
                    for d in &v.declarations {
                        collect_pattern_names(&d.id, out);
                    }
                }
            }
            collect_var_names(&f.body, out);
        }
        Statement::ForOfStatement(f) => {
            if let ForStatementLeft::VariableDeclaration(v) = &f.left {
                if v.kind == VariableDeclarationKind::Var {
                    for d in &v.declarations {
                        collect_pattern_names(&d.id, out);
                    }
                }
            }
            collect_var_names(&f.body, out);
        }
        Statement::SwitchStatement(s) => {
            for c in &s.cases {
                for st in &c.consequent {
                    collect_var_names(st, out);
                }
            }
        }
        Statement::TryStatement(t) => {
            for st in &t.block.body {
                collect_var_names(st, out);
            }
            if let Some(h) = &t.handler {
                for st in &h.body.body {
                    collect_var_names(st, out);
                }
            }
            if let Some(f) = &t.finalizer {
                for st in &f.body {
                    collect_var_names(st, out);
                }
            }
        }
        Statement::LabeledStatement(l) => collect_var_names(&l.body, out),
        _ => {}
    }
}

fn collect_global_var_names(stmt: &Statement, out: &mut Vec<JsString>) {
    // Script-level `var` names, including those nested in blocks.
    collect_var_names(stmt, out);
}
