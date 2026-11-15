use ljs::builtins::native_fn;
use ljs::compiler;
use ljs::value::{Completion, JsString, Value};
use ljs::Interp;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Default, Debug)]
struct Meta {
    flags: Vec<String>,
    includes: Vec<String>,
    features: Vec<String>,
    negative_phase: Option<String>,
    negative_type: Option<String>,
}

fn parse_meta(source: &str) -> Meta {
    let mut meta = Meta::default();
    let start = match source.find("/*---") {
        Some(i) => i + 5,
        None => return meta,
    };
    let end = match source[start..].find("---*/") {
        Some(i) => start + i,
        None => return meta,
    };
    let yaml = &source[start..end];
    if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml) {
        if let Some(map) = value.as_mapping() {
            let list = |key: &str| -> Vec<String> {
                map.get(serde_yaml::Value::String(key.to_string()))
                    .map(|v| match v {
                        serde_yaml::Value::Sequence(seq) => seq
                            .iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect(),
                        serde_yaml::Value::String(s) => vec![s.clone()],
                        _ => Vec::new(),
                    })
                    .unwrap_or_default()
            };
            meta.flags = list("flags");
            meta.includes = list("includes");
            meta.features = list("features");
            if let Some(neg) = map.get(serde_yaml::Value::String("negative".to_string())) {
                if let Some(neg_map) = neg.as_mapping() {
                    meta.negative_phase = neg_map
                        .get(serde_yaml::Value::String("phase".to_string()))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    meta.negative_type = neg_map
                        .get(serde_yaml::Value::String("type".to_string()))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
        }
    }
    meta
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn collect_tests(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut list: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        list.sort_by_key(|e| e.file_name());
        for e in list {
            let path = e.path();
            if path.is_dir() {
                collect_tests(&path, out);
            } else if path.extension().map(|x| x == "js").unwrap_or(false) {
                out.push(path);
            }
        }
    }
}

fn error_name(value: &Value) -> Option<String> {
    let obj = value.as_object_cloned()?;
    let mut cur = Some(obj);
    while let Some(o) = cur {
        if let Some(n) = o.get_hidden("name") {
            if let Some(s) = n.as_string() {
                return Some(s.to_string_lossy());
            }
        }
        cur = o.prototype();
    }
    None
}

fn run_one(
    source: &str,
    include_sources: &[String],
    strict: bool,
    meta: &Meta,
    test262_dir: &Path,
) -> Result<(), String> {
    let mut full = String::new();
    if strict {
        full.push_str("\"use strict\";\n");
    }
    for inc in include_sources {
        full.push_str(inc);
        full.push('\n');
    }
    full.push_str(source);

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, &full, SourceType::default()).parse();
    if meta.negative_phase.as_deref() == Some("parse") {
        if !ret.diagnostics.is_empty() {
            return Ok(());
        }
        // Some early errors are only detected by the compiler.
        match compiler::compile_script(&full, &ret.program) {
            Err(_) => return Ok(()),
            Ok(_) => return Err("Expected parse error, but parsing succeeded".to_string()),
        }
    }
    if !ret.diagnostics.is_empty() {
        return Err(format!("SyntaxError: {:?}", ret.diagnostics.first()));
    }
    let proto = match compiler::compile_script(&full, &ret.program) {
        Ok(p) => p,
        Err(e) => return Err(format!("CompileError: {}", e.message)),
    };
    let mut interp = Interp::new();
    interp.init_core();
    interp.fuel = 20_000_000;
    interp.deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    let done = native_fn("$DONE", 1, |interp, _this, args| {
        interp.done_value = Some(args.first().cloned().unwrap_or(Value::Undefined));
        Ok(Value::Undefined)
    });
    done.set_prototype_raw(Some(interp.realm.intrinsics.function_prototype.clone()));
    interp.realm.global.put_hidden("$DONE", Value::Object(done));
    let result = interp.run_script(proto);
    match result {
        Ok(_) => {
            let _ = interp.run_jobs(100_000);
        }
        Err(Completion::Throw(v)) => {
            if meta.negative_phase.as_deref() == Some("runtime") {
                if let Some(expected) = &meta.negative_type {
                    let actual = error_name(&v).unwrap_or_default();
                    if &actual == expected {
                        return Ok(());
                    }
                    return Err(format!("Expected {} but got {}", expected, actual));
                }
            }
            let msg = if let Some(o) = v.as_object_cloned() {
                let m = o
                    .get_hidden("message")
                    .and_then(|m| m.as_string().map(|s| s.to_string_lossy()))
                    .unwrap_or_default();
                format!("{}: {}", error_name(&v).unwrap_or_default(), m)
            } else {
                v.to_debug_string()
            };
            return Err(format!("Uncaught {}", msg));
        }
        Err(_) => return Err("Uncaught abrupt completion".to_string()),
    }
    if meta.flags.iter().any(|f| f == "async") {
        let mut steps = 0;
        while interp.done_value.is_none() && steps < 200 {
            let before = interp.realm.jobs.len();
            let _ = interp.run_jobs(100_000);
            steps += 1;
            if interp.done_value.is_some() {
                break;
            }
            if interp.realm.jobs.is_empty() && before == 0 {
                break;
            }
        }
        match interp.done_value.clone() {
            None => return Err("Async test did not call $DONE".to_string()),
            Some(v) if v.is_undefined() => {}
            Some(v) => return Err(format!("Async test failure: {}", v.to_debug_string())),
        }
    }
    let _ = test262_dir;
    Ok(())
}

fn main() {
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(real_main)
        .expect("spawn runner");
    let _ = handle.join();
}

fn run_single_mode(path: &Path) {
    let source = read_file(path);
    let meta = parse_meta(&source);
    let skip = meta.flags.iter().any(|f| f == "module" || f == "raw") || {
        let skip_features: HashSet<String> = [
            "Temporal",
            "Intl",
            "ShadowRealm",
            "Atomics",
            "SharedArrayBuffer",
            "decorators",
            "explicit-resource-management",
            "import-defer",
            "source-phase-imports",
            "dynamic-import",
            "import-attributes",
            "regexp-duplicate-named-groups",
            "promise-try",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        meta.features.iter().any(|f| skip_features.contains(f))
    };
    if skip {
        println!("SKIP");
        std::process::exit(0);
    }
    let mut harness_dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    while !harness_dir.join("harness").join("assert.js").exists() {
        match harness_dir.parent() {
            Some(p) => harness_dir = p.to_path_buf(),
            None => {
                println!("FAIL\tmissing harness");
                std::process::exit(1);
            }
        }
    }
    let harness = harness_dir.join("harness");
    let mut includes: Vec<String> = vec!["sta.js".to_string(), "assert.js".to_string()];
    for inc in &meta.includes {
        if !includes.contains(inc) {
            includes.push(inc.clone());
        }
    }
    let mut include_sources = Vec::new();
    for inc in &includes {
        include_sources.push(read_file(&harness.join(inc)));
    }
    let strict = meta.flags.iter().any(|f| f == "onlyStrict");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_one(&source, &include_sources, strict, &meta, &harness_dir)
    }));
    match result {
        Ok(Ok(())) => {
            println!("PASS");
            std::process::exit(0);
        }
        Ok(Err(e)) => {
            println!("FAIL\t{}", e.replace('\n', " "));
            std::process::exit(1);
        }
        Err(_) => {
            println!("FAIL\tengine panic");
            std::process::exit(1);
        }
    }
}

fn real_main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--single") {
        if let Some(path) = args.get(pos + 1) {
            run_single_mode(Path::new(path));
            return;
        }
    }
    let mut root = PathBuf::from("test262/test");
    let mut filter: Option<String> = None;
    let mut limit = usize::MAX;
    let mut verbose = false;
    let mut trace = false;
    let mut skip_features: HashSet<String> = [
        "Temporal",
        "Intl",
        "ShadowRealm",
        "Atomics",
        "SharedArrayBuffer",
        "decorators",
        "explicit-resource-management",
        "import-defer",
        "source-phase-imports",
        "dynamic-import",
        "import-attributes",
        "regexp-duplicate-named-groups",
        "promise-try",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                root = PathBuf::from(&args[i]);
            }
            "--filter" => {
                i += 1;
                filter = Some(args[i].clone());
            }
            "--limit" => {
                i += 1;
                limit = args[i].parse().unwrap_or(usize::MAX);
            }
            "--verbose" => verbose = true,
            "--trace" => trace = true,
            "--all-features" => skip_features.clear(),
            "--skip" => {
                i += 1;
                for f in args[i].split(',') {
                    skip_features.insert(f.to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    let test262_dir = root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("test262"));
    let harness_dir = test262_dir.join("harness");
    let mut files = Vec::new();
    collect_tests(&root, &mut files);
    if let Some(f) = &filter {
        files.retain(|p| p.to_string_lossy().replace('\\', "/").contains(f));
    }
    let total_available = files.len();
    files.truncate(limit);

    let mut pass = 0;
    let mut fail = 0;
    let mut skipped = 0;
    let mut by_dir: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    let started = std::time::Instant::now();

    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
            .to_string();
        let top = rel.split('/').next().unwrap_or("").to_string();
        let source = read_file(path);
        if trace {
            eprintln!("RUN {}", rel);
        }
        let meta = parse_meta(&source);
        let skip = meta.flags.iter().any(|f| f == "module" || f == "raw")
            || meta.features.iter().any(|f| skip_features.contains(f));
        let entry = by_dir.entry(top).or_insert((0, 0, 0, 0));
        if skip {
            skipped += 1;
            entry.3 += 1;
            continue;
        }
        let mut include_sources = Vec::new();
        let mut includes: Vec<String> = vec!["sta.js".to_string(), "assert.js".to_string()];
        for inc in &meta.includes {
            if !includes.contains(inc) {
                includes.push(inc.clone());
            }
        }
        let mut include_error = None;
        for inc in &includes {
            let content = read_file(&harness_dir.join(inc));
            if content.is_empty() && !harness_dir.join(inc).exists() {
                include_error = Some(format!("missing include {}", inc));
            }
            include_sources.push(content);
        }
        if let Some(e) = include_error {
            fail += 1;
            entry.1 += 1;
            failures.push((rel, e));
            continue;
        }
        let strict = meta.flags.iter().any(|f| f == "onlyStrict");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_one(&source, &include_sources, strict, &meta, &test262_dir)
        }));
        let result = match outcome {
            Ok(r) => r,
            Err(_) => Err("engine panic".to_string()),
        };
        match result {
            Ok(()) => {
                pass += 1;
                entry.0 += 1;
            }
            Err(e) => {
                fail += 1;
                entry.1 += 1;
                if verbose {
                    eprintln!("FAIL {}: {}", rel, e);
                }
                if failures.len() < 5000 {
                    failures.push((rel, e));
                }
            }
        }
        if !verbose && (pass + fail) % 500 == 0 {
            eprintln!(
                "progress {}/{} pass={} fail={}",
                pass + fail + skipped,
                files.len(),
                pass,
                fail
            );
        }
    }
    let elapsed = started.elapsed();
    println!(
        "TOTAL {} (available {}, limited {}) | PASS {} | FAIL {} | SKIP {} | PASS_RATE {:.2}% | {:.1}s",
        pass + fail + skipped,
        total_available,
        files.len(),
        pass,
        fail,
        skipped,
        if pass + fail > 0 {
            pass as f64 * 100.0 / (pass + fail) as f64
        } else {
            0.0
        },
        elapsed.as_secs_f64()
    );
    let mut dirs: Vec<_> = by_dir.into_iter().collect();
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (dir, (p, f, _s, sk)) in dirs {
        let total = p + f;
        println!(
            "  {:<45} pass={:<6} fail={:<6} skip={:<5} rate={:.1}%",
            dir,
            p,
            f,
            sk,
            if total > 0 {
                p as f64 * 100.0 / total as f64
            } else {
                0.0
            }
        );
    }
    if !failures.is_empty() {
        let path = "test262-failures.txt";
        let body = failures
            .iter()
            .map(|(n, e)| format!("{}\t{}", n, e))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(path, body);
        println!("failures written to {}", path);
    }
    let _ = Arc::strong_count(&Arc::new(1));
    let _ = JsString::empty();
}
