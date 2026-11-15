use ljs::{compiler, Interp};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.get(1) {
        Some(p) if p == "-h" || p == "--help" => {
            println!(
                "ljs {} - lightweight JavaScript engine",
                env!("CARGO_PKG_VERSION")
            );
            println!("usage: ljs <file.js>");
            return;
        }
        Some(p) if p == "-V" || p == "--version" => {
            println!("ljs {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: ljs <file.js>");
            std::process::exit(2);
        }
    };
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", path, e);
        std::process::exit(2);
    });
    let allocator = Allocator::default();
    let source_type = SourceType::default();
    let ret = Parser::new(&allocator, &source, source_type).parse();
    if !ret.diagnostics.is_empty() {
        for d in &ret.diagnostics {
            eprintln!("SyntaxError: {:?}", d);
        }
        std::process::exit(1);
    }
    let proto = match compiler::compile_script(&source, &ret.program) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("CompileError: {}", e.message);
            std::process::exit(1);
        }
    };
    let mut interp = Interp::new();
    interp.init_core();
    match interp.run_script(proto) {
        Ok(_) => {
            let _ = interp.run_jobs(100_000);
        }
        Err(ljs::value::Completion::Throw(v)) => {
            if let Some(o) = v.as_object_cloned() {
                let msg = o.get_hidden("message");
                let name = o
                    .prototype()
                    .and_then(|p| p.get_hidden("name"))
                    .and_then(|n| n.as_string().map(|s| s.to_string_lossy()));
                eprintln!(
                    "Uncaught {}: {}",
                    name.unwrap_or_else(|| "Object".to_string()),
                    msg.map(|m| m.to_debug_string()).unwrap_or_default()
                );
            } else {
                eprintln!("Uncaught {}", v.to_debug_string());
            }
            std::process::exit(1);
        }
        Err(other) => {
            eprintln!("Uncaught completion: {:?}", other);
            std::process::exit(1);
        }
    }
}
