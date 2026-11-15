# Test262 测试说明与结果

## 1. 获取 Test262

```bash
cd lightweight-js-engine
git clone --depth 1 https://github.com/tc39/test262.git test262
```

目录约定：

```
test262/
  harness/        assert.js、sta.js、compareArray.js 等
  test/           54,089 个 .js 测试文件
```

## 2. 运行方式

### 2.1 进程隔离并行 runner（完整跑分）

```bash
cargo build --release --bin test262
python tools/run_test262.py --workers 24 --timeout 6
```

输出：

- 每 1000 个测试的进度；
- 总分：`TOTAL / PASS / FAIL / SKIP / PASS_RATE`；
- 分类通过率（`annexB`、`built-ins`、`harness`、`intl402`、`language`、`staging`）；
- `test262-failures.txt`：每行 `相对路径<TAB>失败原因`。

脚本行为：

- 对每个测试启动独立进程 `test262 --single <path>`，避免 panic/死循环/内存问题污染整轮；
- 子进程超时 6 秒（可调）；
- 子进程自行解析元数据、includes、features、flags，并返回 `PASS` / `SKIP` / `FAIL`。

### 2.2 内置单进程 runner（调试）

```bash
# 全部（顺序执行，较慢，适合小范围）
./target/release/test262 --filter built-ins/Array --limit 500

# 显示失败
./target/release/test262 --verbose

# 追踪正在运行的测试（定位 panic/卡死）
./target/release/test262 --limit 3000 --trace

# 单个测试
./target/release/test262 --single test262/test/language/expressions/addition/bigint-arithmetic.js
```

内置 runner 使用：

- 512MB 线程栈；
- `catch_unwind` 捕获引擎 panic；
- 执行 fuel = 20,000,000；
- 每个测试 5 秒 deadline（通过 `Interp.deadline`，在属性访问/调用路径检查）。

## 3. 测试范围与跳过策略

Test262 的 `features` 字段用于标记可选/提案特性。默认跳过以下尚未实现或超出 ECMAScript 核心范围的特性：

| 特性 | 说明 |
|---|---|
| `Temporal` | 日期时间提案，未实现 |
| `Intl` | ECMA-402 国际化标准，未实现 |
| `ShadowRealm` | 提案 |
| `Atomics` / `SharedArrayBuffer` | 并发原语，仅部分实现 |
| `decorators` | 装饰器提案 |
| `explicit-resource-management` | `using` / `await using` 提案 |
| `import-defer` / `source-phase-imports` | 模块提案 |
| `dynamic-import` / `import-attributes` | 模块加载未实现 |
| `regexp-duplicate-named-groups` | 正则提案 |
| `promise-try` | `Promise.try` 提案（内建已提供，但元数据默认跳过） |
| `module` / `raw` flags | 模块测试与 raw 测试 |

此外：

- `staging/` 为提案测试，不属于已标准化 ECMAScript 核心；
- `intl402/` 属于 ECMA-402，不属于 ECMAScript 核心；
- 未实现 `Intl`/`Temporal` 是明确的工程范围选择，README 中已注明。

因此报告两个通过率：

- **核心通过率** = `PASS / (PASS + FAIL)`，只统计已启用特性范围内的测试（推荐用于衡量 ECMAScript 实现完整度）；
- **全量通过率** = `PASS / 54,089`，把跳过的特性也计入分母。

## 4. 基线结果

运行环境：

- Windows 11
- Rust stable，`cargo build --release`
- `python tools/run_test262.py --workers 24 --timeout 6`

结果（`docs/TEST262.md` 更新时以最新 `pyfull*.txt` 为准）：

| 指标 | 数值 |
|---|---:|
| 测试文件总数 | 54,089 |
| PASS | 28,082 |
| FAIL | 16,214 |
| SKIP | 9,793 |
| **核心通过率** | **63.40%** |
| 全量通过率 | 51.92% |
| 核心 ECMAScript（排除 intl402/staging） | 65.94% |
| 耗时 | 428 秒（24 workers） |

分类：

| 分类 | PASS | FAIL | SKIP | 通过率 |
|---|---:|---:|---:|---:|
| `language` | 15,249 | 6,843 | 2,023 | 69.0% |
| `built-ins` | 11,622 | 6,690 | 5,570 | 63.5% |
| `annexB` | 452 | 630 | 4 | 41.8% |
| `harness` | 109 | 8 | 0 | 93.2% |
| `staging` | 627 | 783 | 85 | 44.5% |
| `intl402` | 23 | 1,260 | 2,111 | 1.8% |

> 说明：上表为加入 `with`、`switch` 内 `break`、类字段早期错误、`new.target` 构造校验、`TypedArray.from/of` 等修复后的结果；完整日志见 `docs/test262-latest.txt`，失败明细见 `test262-failures.txt`。

### 严格模式说明

内置 runner 目前对每个测试文件只执行一种模式：

- `flags: [onlyStrict]` → 严格模式；
- 其余 → 非严格模式（sloppy）。

Test262 官方 harness 会对没有 `onlyStrict`/`noStrict` 标记的测试同时执行 sloppy 与 strict 两个变体，并要求两者都通过。因此本项目的通过率高于“双变体严格通过率”。后续可通过给 runner 增加 `--strict-variants` 来测量双变体结果；当前基线用于衡量引擎在单变体下的规范覆盖度。

## 5. 失败分类（基线）

按失败原因聚合：

| 原因 | 数量 |
|---|---:|
| 未捕获异常（多为 `Test262Error` 断言失败） | 7,148 |
| `TypeError` | 3,562 |
| `ReferenceError` | 1,776 |
| 断言/期望不符 | 1,525 |
| 异步测试失败 | 941 |
| 编译错误 | 496 |
| 超时 | 283 |
| `RangeError` | 220 |
| 语法错误 | 125 + 42 |
| 引擎 panic | 38 |

主要失败目录：

| 目录 | 失败数 | 主要原因 |
|---|---:|---|
| `language/statements/class` | 1,575 | 类早期错误、私有方法/访问器、静态块、`super` 细节 |
| `language/expressions/class` | 1,418 | 同上 |
| `built-ins/TypedArray/prototype` | 1,053 | 部分原型方法、species、resizable buffer |
| `built-ins/RegExp/property-escapes` | 611 | regress 不支持 Unicode property escapes |
| `built-ins/Array/prototype` | 490 | 极端 length/空洞/Proxy 交互 |
| `built-ins/Iterator/prototype` | 374 | Iterator helper 规范细节 |
| `built-ins/Object/defineProperty` | 350 | 描述符完整校验 |
| `language/expressions/object` | 350 | 方法定义/早期错误 |
| `built-ins/String/prototype` | 318 | 部分字符串方法与正则交互 |
| `language/expressions/assignment` | 299 | 解构/复合赋值边界 |
| `annexB/language/eval-code` | 252 | eval 变量环境细节 |
| `language/expressions/async-generator` | 252 | async generator 边界 |
| `intl402/*` | 1,260 | 未实现 Intl |

## 6. 复现实验

```bash
# 1) 构建
cargo build --release --bin test262

# 2) 跑全量（约 10 分钟，24 并发）
python tools/run_test262.py --workers 24 --timeout 6 | tee pyfull.txt

# 3) 查看失败
head -50 test262-failures.txt

# 4) 跑某一分类
python tools/run_test262.py --filter language/expressions/class --workers 16

# 5) 单个测试（含详细错误）
./target/release/test262 --single test262/test/language/statements/class/elements/fields-duplicate-privatenames.js
```

## 7. 结论

- 在已启用的 ECMAScript 核心特性范围内，通过率 **63.40%**，达到 README 要求的 60%。
- 若把未实现的 `Intl`/`Temporal`/提案特性计入分母，全量通过率为 50.94%；差距主要来自这些明确排除的领域。
- 后续提升优先级：TypedArray 原型完整性、类私有方法/访问器、Iterator helpers、RegExp Unicode property escapes、Array 极端 length 语义、严格模式双变体。
