# 性能 Benchmark

## 1. 方法

- 构建：`cargo build --release`（opt-level=3, thin LTO, codegen-units=1）。
- 驱动：`python tools/bench.py --runs 5`。
- 每个 benchmark 连续执行 5 次，取中位数；测量范围包含进程启动、脚本解析、编译、执行与结果输出。
- 计时使用 `time.perf_counter()`（高精度单调时钟）。
- 运行环境：Windows 11，Rust stable，Release 构建。

```bash
cargo build --release
python tools/bench.py --runs 5 --json bench-results.json
```

## 2. Benchmark 脚本

| 文件 | 内容 | 主要负载 |
|---|---|---|
| `bench/fib.js` | `fib(30)` 递归 | 函数调用、栈帧、算术、条件分支 |
| `bench/loop.js` | 3,000,000 次数值循环 | 局部变量、位运算、循环跳转 |
| `bench/property.js` | 2,000,000 次对象属性读取（4 个属性） | 属性查找、字符串键、对象模型 |
| `bench/array.js` | 200,000 元素 push/读取/map | 数组存储、原生回调、数组方法 |
| `bench/string.js` | 50,000 次字符串拼接 | UTF-16 字符串、内存分配 |
| `bench/closure.js` | 2,000,000 次闭包调用 | 环境链、upvalue、函数调用 |
| `bench/object.js` | 200,000 次对象字面量创建与方法调用 | 对象分配、方法调用 |

## 3. 结果（中位数）

| Benchmark | 中位数（秒） | 备注 |
|---|---:|---|
| `fib` (fib 30) | 4.7270 | 约 270 万次函数调用 |
| `loop` (3M) | 2.6334 | 约 114 万次迭代/秒 |
| `property` (8M gets) | 2.1006 | 约 381 万次属性读取/秒 |
| `array` (200k) | 1.0146 | push + 遍历 + map |
| `string` (50k) | 0.5488 | UTF-16 拼接 |
| `closure` (2M) | 3.1908 | 约 63 万次闭包调用/秒 |
| `object` (200k) | 0.5486 | 对象字面量 + 方法调用 |

原始数据：`bench-results.json`。

### 各次运行（秒）

| Benchmark | Run 1 | Run 2 | Run 3 | Run 4 | Run 5 |
|---|---:|---:|---:|---:|---:|
| array | 1.0146 | 0.9959 | 1.1004 | 1.0049 | 1.0334 |
| closure | 3.1711 | 3.1908 | 3.2698 | 3.3247 | 3.1250 |
| fib | 4.0403 | 4.4048 | 4.7270 | 5.0286 | 5.1262 |
| loop | 3.4030 | 3.6290 | 2.6334 | 2.3614 | 2.4907 |
| object | 0.5486 | 0.5310 | 0.5997 | 0.5561 | 0.5382 |
| property | 2.5909 | 2.4593 | 2.0044 | 2.1006 | 1.8993 |
| string | 0.5488 | 0.5720 | 0.7871 | 0.5306 | 0.5152 |

## 4. 分析

- **函数调用是主要瓶颈**：`fib`/`closure` 每个调用都需要创建 `Frame`、扩展参数、维护 env 链、检查调用深度与 deadline。后续可通过：
  - 复用 `Frame`/参数 Vec；
  - 对非捕获函数省略环境分配；
  - 调用点内联缓存（IC）。
- **属性访问**：每次 `get_property` 需要 `Rc<RefCell>` 借用、`PropertyKey` 构造、原型链遍历。可增加：
  - 字符串驻留（interned key）；
  - 单态属性 inline cache；
  - 数组索引快速路径（当前已有，但可进一步减少边界检查）。
- **数组方法**：`Array.prototype.map` 通过原生回调逐元素调用，回调 frame 分配开销明显；可增加快速路径（无稀疏/无副作用时批量处理）。
- **字符串拼接**：`JsString` 为 UTF-16 `Vec<u16>`，每次 `+=` 复制；可引入 rope/Builder 优化长字符串构建。
- **启动开销**：每个进程初始化全部内建对象，约占小型脚本总时间的显著比例；嵌入场景可缓存 Realm 模板。

## 5. 与其他引擎的对比

本仓库不内置其他引擎的对比，以避免不公平的配置差异。若需对比，可使用同一份 `bench/*.js`，但脚本使用 `print()` 作为输出函数；Node.js 需要注入 `globalThis.print = console.log`：

```bash
node -e "globalThis.print=console.log; require('./bench/fib.js')"
```

本项目的目标不是与 JIT 引擎（V8/JSC）比拼峰值吞吐，而是提供：
- 无 JIT、纯 Rust、可在任意平台编译；
- 启动快、内存模型简单、易于嵌入；
- 可预测的尾延迟（无 GC 停顿，使用引用计数）。

## 6. 复现实验

```bash
cargo build --release
python tools/bench.py --runs 5 --json bench-results.json
```

在 CI 或不同机器上，绝对时间会变化；建议在同一机器、同一电源模式、空载条件下比较。
