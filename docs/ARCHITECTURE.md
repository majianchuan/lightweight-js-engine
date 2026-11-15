# 架构与实现说明

本文描述 `ljs` 的整体设计、关键数据结构与执行流程，便于二次开发和性能调优。

## 1. 总体流水线

```
Source ──OXC──▶ AST ──Compiler──▶ FunctionProto(bytecode) ──VM──▶ Value/Completion
                                      │
                                      ├─ Constant Pool
                                      ├─ Handler Table (try/catch/finally)
                                      └─ Nested Function Protos
```

- **Parser**：`oxc_parser` 生成 AST。OXC 只负责语法解析；所有作用域、闭包、类、解构和早期错误检查由编译器完成。
- **Compiler**：单遍 AST → 字节码，同时在编译期完成：
  - 变量槽分配（`depth` + `slot`）；
  - 闭包捕获的外层作用域快照（`OuterScope`）；
  - 循环/标签的跳转回填；
  - `try/catch/finally` 处理器表；
  - 类字段、私有名、静态块展开；
  - 解构模式展开为属性读取/迭代器操作。
- **VM**：栈式解释器，`Frame` 保存函数原型、环境、操作数栈、临时槽、PC、`this`、`new.target`、`home object`、异常处理器和完成记录。

## 2. 值表示

`src/value.rs`：

```rust
pub enum Value {
    Undefined, Null, Bool(bool), Number(f64), BigInt(...),
    String(JsString), Symbol(SymbolValue), Object(JsObject),
}
```

- 字符串统一为 UTF-16（`JsString`），保证 `length`、`charCodeAt`、代理对和正则索引符合 ECMAScript。
- `PropertyKey` 为字符串或 Symbol；数组索引在属性操作中单独快速路径处理。
- 数字转字符串使用 `ryu-js`，保证与 ECMAScript `Number::toString` 一致（含 `-0`、指数格式）。

## 3. 对象模型

`src/object.rs`：

```rust
pub struct JsObject(pub Rc<RefCell<ObjectData>>);

pub struct ObjectData {
    pub prototype: Option<JsObject>,
    pub extensible: bool,
    pub properties: IndexMap<PropertyKey, Property>,
    pub kind: ObjectKind,
    pub private: IndexMap<u64, Property>,
}
```

`ObjectKind` 区分普通对象、数组、函数、绑定函数、Proxy、String/Number/Boolean/Symbol/BigInt 包装对象、Arguments、RegExp、Date、Map/Set、Promise、Generator/AsyncGenerator、ArrayBuffer/SharedArrayBuffer、TypedArray、DataView、迭代器等。

属性操作在 `src/ops.rs` 中实现：

- `get_property` / `set_property` / `define_own_property` / `delete_property` / `own_property_keys`；
- 数据属性与访问器属性；
- 数组 `length` 的自动维护、删除/截断、索引边界；
- 字符串包装对象的索引属性；
- TypedArray 的整数索引、越界/分离缓冲区行为；
- Proxy 的 13 个陷阱（`src/proxy.rs`）；
- `Object.freeze/seal/preventExtensions` 与完整性级别。

## 4. 环境与闭包

`src/env.rs`：

```rust
pub struct EnvRef(pub Rc<Env>);

pub struct Env {
    pub parent: Option<EnvRef>,
    pub kind: EnvKind,
}

pub enum EnvKind {
    Declarative(RefCell<Vec<Option<Value>>>),
    Object(JsObject),   // with
}
```

- 编译器为每个函数/块计算 `depth` 与 `slot`；运行时通过 `LoadVar { depth, slot }` 逐级取环境。
- 闭包创建时记录 `OuterScope`（绑定表 + 相对环境偏移），实现按需捕获。
- `with` 使用对象环境；编译器在 `with_depth > 0` 时生成 `LoadName/StoreName/TypeOfName`，运行时优先在 `with` 对象上查找，未命中再回退到局部槽/全局。
- 循环的 `let` 绑定在每轮迭代创建新环境，保证闭包捕获的是当轮值。

## 5. 字节码

`src/bytecode.rs` 中 `Op` 覆盖：

- 栈/常量/局部变量/全局变量；
- 算术、比较、位运算、逻辑短路；
- 属性读写（命名/计算/`super`）；
- 调用、构造、`new.target`、`this`；
- 对象/数组字面量与展开；
- 闭包、环境创建/销毁；
- 类定义（构造器、原型、方法、字段、静态块、私有名）；
- 迭代器（同步/异步）、`for-in`、`for-of`、`for await-of`；
- 异常（`Throw`、handler 表、`EndFinally`、完成记录）；
- 生成器（`Yield`）与异步（`Await`）。

`FunctionProto` 同时保存：

- `code: Vec<Op>`；
- `constants: Vec<Constant>`；
- `handlers: Vec<Handler>`；
- 参数/变量槽数量、函数名、`length`、严格模式、箭头/生成器/异步标志、home object 需求；
- 嵌套函数原型表。

## 6. 执行与异常

`Interp::execute_frame`：

1. 按 `Resume`（Normal/Value/Throw/Return）初始化栈/完成记录；
2. 循环取指执行，维护 `fuel` 与截止时间；
3. 遇到 `Throw` 查找最近 handler，跳转到 catch/finally；
4. `finally` 通过 `Completion` + `EndFinally` 恢复原始完成（normal/return/throw/break/continue）；
5. 函数返回时恢复调用者的 frame、`this`、`new.target`、home object；
6. 生成器/异步函数在 `Yield`/`Await` 时保存 frame，稍后恢复。

`call_function` 支持：

- 普通函数、箭头函数、绑定函数、类构造器（含派生类 `super()`）；
- 原生函数（`NativeFunction`）；
- Proxy 的 `apply`；
- 生成器/async/async generator 的对象创建与恢复；
- 调用深度上限与栈溢出错误。

## 7. 异步与微任务

`src/realm.rs`：

```rust
pub enum Job {
    PromiseReaction { reaction, argument },
    PromiseResolveThenable { ... },
    ResumeAsync { state, value, is_error },
    AsyncGenResume { generator, value, is_error },
}
```

- Promise 的 `then` 注册 reaction；resolve/reject 只调度匹配类型的 reaction（fulfill/reject 分开），避免同时调用两个回调。
- `async function` 在 `Await` 时挂起 frame，Promise settle 后以 `ResumeAsync` 恢复。
- `async generator` 维护请求队列（next/throw/return）、当前挂起请求和 frame；`yield` 的值会先 `Await`（符合 async generator 语义），再 resolve 对应请求的 Promise。
- `for await...of` 编译为 `AsyncIterInit` + `AsyncIterNext` + `Await`，同时支持异步迭代器和同步迭代器回退。
- CLI 在脚本结束后调用 `run_jobs()` 直到队列为空，保证 Promise/async 回调执行完毕。

## 8. 内建对象组织

- `builtins.rs`：全局对象、`Object`、`Function`、`Array` 基础、`String`/`Number`/`Boolean`/`Symbol`/`BigInt`、错误、`Math`、`eval`、`$262` 测试钩子。
- `builtins_array.rs`：Array 原型方法、`Array.from/of`、species 近似。
- `builtins_string.rs`：字符串方法、UTF-16 辅助。
- `builtins_regexp.rs`：RegExp 构造/执行、flags、`@@match/replace/search/split/matchAll`。
- `builtins_json.rs`：JSON 解析/序列化（保持插入顺序）。
- `builtins_promise.rs`：Promise 构造、组合子、微任务循环。
- `builtins_generator.rs` / `builtins_async.rs` / `builtins_asyncgen.rs`：生成器与异步。
- `builtins_collections.rs`：Map/Set/WeakMap/WeakSet 及迭代器。
- `builtins_typedarray.rs`：ArrayBuffer/DataView/TypedArray。
- `builtins_proxy.rs`：Proxy/Reflect。
- `builtins_iterator.rs`：Iterator 构造器与 helper。
- `builtins_date.rs`：Date 解析/格式化/时区（本地时间按 UTC 处理以保证确定性）。

## 9. 测试与鲁棒性

- 内置 runner（`src/bin/test262.rs`）：
  - 解析 YAML frontmatter、includes、flags、negative；
  - 默认 `sta.js` + `assert.js`；
  - 支持 `--filter`、`--limit`、`--verbose`、`--trace`、`--single`；
  - 每个测试独立 Realm；
  - 512MB 线程栈、`catch_unwind`、fuel（20,000,000）与 5 秒 deadline。
- 进程隔离 runner（`tools/run_test262.py`）：
  - 每个测试一个子进程，超时 6 秒；
  - 并行 24 workers；
  - 自动汇总分类统计与失败明细。

## 10. 已知边界

- 无 GC：`Rc` 循环引用不会回收。
- `Intl` / `Temporal` 未实现。
- `regress` 不支持 Unicode property escapes 与 `v` flag 全部集合运算。
- `eval` 与 `with` 使用近似实现，极端动态作用域场景可能偏离规范。
- 私有字段使用内部 Symbol 存储，`Object.getOwnPropertySymbols` 可能观察到（正常枚举不可见）。
- 直接以源码字符串调用 `Function` 构造器时会重新解析；未做字节码缓存。
