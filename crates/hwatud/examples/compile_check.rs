//! Bisect helper: find converted rules WebKit's compiler rejects.
//! Usage: cargo run --example compile_check -p hwatud -- <list.txt>...
//! Not shipped; developer tool for filter-converter debugging.

use gtk::glib;
use serde_json::Value;

#[path = "../src/abp.rs"]
#[allow(dead_code)]
mod abp;

fn main() {
    gtk::init().unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut text = String::new();
    for path in &args {
        text.push_str(&std::fs::read_to_string(path).expect("read list"));
        text.push('\n');
    }
    let converted = abp::convert(text.lines());
    let rules: Vec<Value> = serde_json::from_str(&converted.json).unwrap();
    println!("total rules: {}", rules.len());

    let dir = std::env::temp_dir().join("hwatu-compile-check");
    let _ = std::fs::create_dir_all(&dir);
    let store = webkit6::UserContentFilterStore::new(&dir.to_string_lossy());

    let ctx = glib::MainContext::default();
    let compile = |rules: &[Value]| -> Result<(), String> {
        let json = serde_json::to_string(rules).unwrap();
        let bytes = glib::Bytes::from_owned(json.into_bytes());
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let r2 = result.clone();
        store.save("bisect", &bytes, gtk::gio::Cancellable::NONE, move |res| {
            r2.borrow_mut()
                .replace(res.map(|_| ()).map_err(|e| e.to_string()));
        });
        while result.borrow().is_none() {
            ctx.iteration(true);
        }
        let out = result.borrow_mut().take().unwrap();
        out
    };

    // Recursive bisect to find every failing rule.
    fn bisect(
        rules: &[Value],
        compile: &dyn Fn(&[Value]) -> Result<(), String>,
        bad: &mut Vec<Value>,
    ) {
        if compile(rules).is_ok() {
            return;
        }
        if rules.len() == 1 {
            bad.push(rules[0].clone());
            return;
        }
        let mid = rules.len() / 2;
        bisect(&rules[..mid], compile, bad);
        bisect(&rules[mid..], compile, bad);
    }

    let mut bad = Vec::new();
    bisect(&rules, &compile, &mut bad);
    println!("failing rules: {}", bad.len());
    for r in bad.iter().take(50) {
        println!("{}", serde_json::to_string(r).unwrap());
    }
}
