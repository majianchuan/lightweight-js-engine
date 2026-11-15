# Lightweight JS Engine (`ljs`)

一个使用 **Rust** 从零实现的轻量级 JavaScript 执行引擎，包含自己的字节码编译器、栈式虚拟机、对象模型、环境/闭包系统、内建对象与异步任务队列。解析器使用社区成熟的高性能 **OXC Parser**，正则表达式使用 **regress**，其余执行语义由本项目实现，而不是包装 Boa / QuickJS / V8。

目标：在 Linux / macOS / Windows 上提供可嵌入、启动快、依赖少的 JS 执行能力，并通过 ECMAScript Test262 测试集。

---

## 1. 快速开始

### 环境要求

- Rust 1.70+（使用 stable 工具链即可）
- Git（获取 Test262 时需要）
- Linux / macOS / Windows

### 构建与运行

```bash
# 构建 release 版 CLI
cargo build --release

# 执行一个 JS 文件
./target/release/ljs script.js

# Windows
.\target\release\ljs.exe script.js
```

CLI 行为：

- 脚本按非严格模式（sloppy mode）执行；脚本内可使用 `"use strict"`。
- 执行结束后会清空微任务队列（Promise / async / async generator）。
- 未捕获异常会打印 `Uncaught <ErrorName>: <message>` 并以非零状态退出。

示例：

```js
// demo.js
class Counter {
  #n = 0;
  inc() { return ++this.#n; }
}
const c = new Counter();
console.log(c.inc(), c.inc());

async function* gen() {
  yield 1;
  yield await Promise.resolve(2);
}
(async () => {
  for await (const v of gen()) console.log("async", v);
})();
```

### 运行 Test262

仓库约定将 Test262 克隆到项目根目录的 `test262/`：

```bash
git clone --depth 1 https://github.com/tc39/test262.git test262
```

两种运行方式：

```bash
# 1) 内置 Rust runner：单进程、快速冒烟/调试
cargo build --release --bin test262
./target/release/test262 --filter built-ins/Array --limit 500
./target/release/test262 --verbose

# 2) 进程隔离的并行 runner：用于完整跑分（推荐）
python tools/run_test262.py --workers 24 --timeout 6
```

`tools/run_test262.py` 会为每个测试启动独立进程，避免单个测试的 panic、死循环或内存问题影响整个跑分，并输出分类统计与失败列表 `test262-failures.txt`。

---

## 2. 已实现能力

### 语言核心

- 变量：`var` / `let` / `const`，块级作用域、TDZ、闭包、提升。
- 表达式与运算符：完整算术 / 位运算 / 比较 / 逻辑 / 三元 / 逗号 / `typeof` / `instanceof` / `in` / `delete` / `void`。
- 语句：`if` / `switch` / `for` / `for-in` / `for-of` / `for await-of` / `while` / `do-while` / `try-catch-finally` / `throw` / `return` / `break` / `continue` / 标签语句 / `with`（非严格）。
- 函数：函数声明/表达式、箭头函数、默认参数、剩余参数、解构参数、`arguments`、`this`、`new.target`、`Function` 构造器、`eval`（直接/间接，含 `eval` 作用域近似）。
- 对象与数组：对象字面量（含计算属性、getter/setter、简写、展开）、数组字面量（elisions、spread）、解构赋值、属性描述符、`Object` 全套常用 API。
- 类：`class` 声明/表达式、继承、`super` 调用与成员、静态成员、实例字段、静态字段、私有字段/私有方法（基于内部私有名）、静态初始化块、getter/setter、计算属性名。
- 生成器与异步：`function*`、`yield`、`yield*`、`async function`、`await`、`async function*`、`for await...of`、异步迭代协议、Promise 微任务队列。
- 迭代协议：同步/异步迭代器、`Symbol.iterator` / `Symbol.asyncIterator`、生成器委托、迭代器关闭。

### 内建对象（部分）

- `Object`、`Function`、`Array`、`String`、`Number`、`Boolean`、`Symbol`、`BigInt`、`Math`、`JSON`、`Date`、`RegExp`。
- `Map` / `Set` / `WeakMap` / `WeakSet`。
- `Promise` 及 `all` / `allSettled` / `any` / `race` / `try` / `withResolvers`。
- `ArrayBuffer` / `SharedArrayBuffer`（基础）/ `DataView` / 全部 TypedArray 构造器及常用原型方法。
- `Proxy` / `Reflect`。
- `Iterator` 及其 helper 方法（部分）。
- `Error` 系列、`Intl` 与 `Temporal` 未完整实现（见“范围与限制”）。

### 引擎特性

- 自有字节码指令集与寄存器/栈混合执行模型（临时槽 + 操作数栈）。
- 自有属性描述符与普通对象/数组/字符串/函数/Proxy/TypedArray 等 exotic 行为。
- 声明式环境链 + 闭包 upvalue 捕获，支持块级作用域、循环 per-iteration 绑定。
- 异常处理器表、`try/catch/finally`、`break/continue` 跨作用域清理。
- 生成器帧挂起/恢复；async/await 与 Promise 作业队列；async generator 请求队列。
- 执行燃料（fuel）与截止时间（deadline）保护，避免测试/嵌入场景中的死循环拖垮宿主。
- 无 unsafe 的纯 safe Rust 实现（解析器依赖 OXC 自身实现除外）。

---

## 3. 架构

```
源码
  │  OXC Parser (AST)
  ▼
Compiler (src/compiler.rs)
  │  作用域分析 / 闭包捕获 / 类与解构展开 / 异常表
  ▼
Bytecode (src/bytecode.rs: Op, FunctionProto, Constant, Handler)
  │
  ▼
VM (src/vm.rs: Interp / Frame / execute_frame / call_function)
  │  ├── 对象模型 (src/object.rs, src/value.rs)
  │  ├── 环境与闭包 (src/env.rs)
  │  ├── 抽象操作 (src/ops.rs, src/proxy.rs)
  │  ├── Realm/Intrinsics/Job Queue (src/realm.rs)
  │  └── 内建对象 (src/builtins*.rs)
  ▼
结果 / 未捕获异常
```

关键设计：

1. **字节码而非树遍历**：编译期完成变量槽分配、标签/循环跳转回填、`try/finally` 处理器表生成，运行期只做取指与栈操作。
2. **自有对象模型**：`JsObject` 内部使用 `Rc<RefCell<ObjectData>>`，属性保持插入顺序（`indexmap`），完整实现数据/访问器属性、可枚举/可写/可配置以及数组/字符串/参数对象/TypedArray 的 exotic 语义。
3. **环境链**：`EnvRef` 为声明式环境（slot 数组）或 `with` 对象环境；编译期计算相对深度，闭包通过 `env_depth` 捕获外层绑定。
4. **异步调度**：Promise reaction、`ResumeAsync`、`AsyncGenResume` 统一进入 Realm 的微任务队列，由 `run_jobs` 驱动，直到队列为空。
5. **测试鲁棒性**：每个测试独立 Realm；内置 runner 使用 `catch_unwind` + fuel；并行 runner 进一步使用进程隔离与超时。

更多细节见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

---

## 4. Test262 结果

在本机 Windows 11 + Rust release 构建下，使用 `tools/run_test262.py --workers 24 --timeout 6` 完整运行 54,089 个测试文件：

| 指标 | 数值 |
|---|---:|
| 测试文件总数 | 54,089 |
| 通过（PASS） | 28,082 |
| 失败（FAIL） | 16,214 |
| 跳过（SKIP，未实现的可选特性） | 9,793 |
| **通过率（PASS / (PASS + FAIL)）** | **63.40%** |
| 通过率（PASS / 全部文件） | 51.92% |
| 核心 ECMAScript 通过率（排除 `intl402/` 与 `staging/`） | 65.94% |

> 上表为加入类早期错误、`with`、`switch` 内 `break`、`new.target` 校验、`TypedArray.from/of` 等修复后的基线；完整日志见 `docs/test262-latest.txt`，方法与失败分类见 `docs/TEST262.md`。

已跳过的可选/未实现特性包括：`Temporal`、`Intl`（ECMA-402 单独标准）、`ShadowRealm`、`Atomics`/`SharedArrayBuffer` 高级部分、decorators、explicit resource management、import attributes / dynamic import、RegExp duplicate named groups、`Promise.try` 等。跳过由 Test262 元数据中的 `features` 字段驱动。

分类通过率（同一轮）：

| 分类 | PASS | FAIL | SKIP | 通过率 |
|---|---:|---:|---:|---:|
| `language` | 15,249 | 6,843 | 2,023 | 69.0% |
| `built-ins` | 11,622 | 6,690 | 5,570 | 63.5% |
| `annexB` | 452 | 630 | 4 | 41.8% |
| `harness` | 109 | 8 | 0 | 93.2% |
| `staging`（提案） | 627 | 783 | 85 | 44.5% |
| `intl402`（ECMA-402） | 23 | 1,260 | 2,111 | 1.8% |

完整的运行方法、特性范围与失败分类见 [docs/TEST262.md](docs/TEST262.md)。

---

## 5. 性能 Benchmark

项目提供可复现的基准脚本（`bench/`）和驱动脚本（`tools/bench.py`）：

```bash
python tools/bench.py --runs 5
```

基准覆盖：递归调用（fib）、数值循环、属性访问、数组操作、字符串拼接、闭包调用、对象创建。结果与运行环境见 [docs/BENCHMARK.md](docs/BENCHMARK.md)。

---

## 6. 项目结构

```
src/
  main.rs              CLI 入口
  lib.rs               模块导出
  value.rs             Value / JsString / PropertyKey / Symbol / 数字格式化
  object.rs            JsObject、属性描述符、函数/数组/TypedArray/Promise 等对象数据
  env.rs               环境链与闭包
  realm.rs             Realm、Intrinsics、全局对象、微任务队列
  bytecode.rs          操作码、函数原型、常量、异常处理器
  compiler.rs          OXC AST -> 字节码编译器
  vm.rs                解释器、Frame、调用/构造、生成器/异步
  ops.rs               规范抽象操作（属性、转换、迭代、数组 exotic）
  proxy.rs             Proxy 陷阱
  builtins*.rs         全局对象与内建对象实现
  bin/test262.rs       Test262 Rust runner（单进程 / --single）
tools/
  run_test262.py       进程隔离的并行 Test262 runner
  bench.py             基准驱动
bench/                 基准 JS 脚本
docs/
  ARCHITECTURE.md      架构与实现说明
  TEST262.md           测试方法与结果
  BENCHMARK.md         性能数据
```

---

## 7. 范围与限制

- **不是** 浏览器/Node 兼容运行时：没有 DOM、Node API、模块加载器、`setTimeout` 等宿主 API。
- `Intl`（ECMA-402）与 `Temporal` 未实现，相关测试默认跳过；`intl402/` 与提案类 `staging/` 测试不计入核心 ECMAScript 通过率。
- 正则表达式由 `regress` 提供，支持常见 ES 正则特性；Unicode property escapes、部分 `v` flag 集合运算与最新提案存在差距。
- `eval` 采用编译期/运行期近似作用域实现，极端动态作用域场景可能与规范有差异。
- 私有字段通过内部唯一私有名 + Symbol 属性实现，反射 API 中可能观察到内部 Symbol（正常 `Object.keys`/`for-in` 不可见）。
- 直接 `with`、`arguments`、标签等边界语义已覆盖常见用例，但并非 100% 规范完备。
- 未实现真正的增量 GC；对象使用 `Rc` 引用计数，循环引用会泄漏。适合短生命周期脚本与嵌入执行场景。

---

## 8. 开发与测试

```bash
cargo build
cargo test
cargo build --release
cargo build --release --bin test262
python tools/run_test262.py --filter language/expressions/addition
```

代码风格：`cargo fmt`；静态检查：`cargo clippy`。

第三方依赖与用途：

| crate | 用途 |
|---|---|
| `oxc_parser` / `oxc_ast` / `oxc_span` / `oxc_allocator` | JavaScript 解析与 AST |
| `regress` | ECMAScript 正则表达式引擎 |
| `ryu-js` | 符合 ECMAScript 的数字转字符串 |
| `num-bigint` | BigInt |
| `chrono` | Date 解析/格式化 |
| `serde_json` | JSON.parse/stringify |
| `indexmap` | 保持属性插入顺序 |
| `unicode-id-start` / `unicode-normalization` | 标识符与字符串规范化 |

除上述基础库外，语言语义、字节码、VM、对象模型、内建对象均为本项目自研实现。
