//! Times the CLI against a tracker holding a synthetic plan: the import, then medians of reads and one-field edits,
//! as the import left the store and again after `git gc`; optionally after many single-field batches.
//!
//! `mise run bench:store -- [--items <n>] [--runs <n>] [--batches <n>]`

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use acyclic_cli::{Env, execute};

fn main() {
    let mut items = 3_500;
    let mut runs = 5;
    let mut batches = 0;
    let args: Vec<String> = std::env::args().skip(1).collect();
    for pair in args.chunks(2) {
        let value = pair.get(1).and_then(|v| v.parse().ok());
        match (pair[0].as_str(), value) {
            ("--items", Some(n)) => items = n,
            ("--runs", Some(n)) => runs = n,
            ("--batches", Some(n)) => batches = n,
            _ => {
                eprintln!("usage: bench_store [--items <n>] [--runs <n>] [--batches <n>]");
                std::process::exit(2);
            },
        }
    }
    let dir = tempfile::TempDir::new().expect("a temporary directory");
    let env = Env {
        vars: [("ACYCLIC_TRACKER", "tracker"), ("HOME", "/nonexistent")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into(),
        cwd: dir.path().to_path_buf(),
    };
    let plan = acyclic_fixtures::generate(items, 7);
    std::fs::write(dir.path().join("plan.json"), plan.to_json()).expect("the plan is written");
    let item = plan.items.iter().rev().find(|i| i.state != acyclic_core::import::State::Done);
    let item = &item.expect("an open item").key;

    run(&env, "init tracker --replica bench");
    println!("{items} items, medians of {runs}");
    report("import", &[time(&env, "import plan.json --as-user")]);
    report("import again (no change)", &[time(&env, "import plan.json --as-user")]);
    measure(&env, item, runs, "as imported");
    if batches > 0 {
        for n in 0..batches {
            run(&env, &format!("edit {item} --notes batch-{n} --as-user"));
        }
        measure(&env, item, runs, &format!("after {batches} more batches"));
    }
    gc(&dir.path().join("tracker"));
    measure(&env, item, runs, "after git gc");
}

fn measure(env: &Env, item: &str, runs: usize, label: &str) {
    println!("-- {label}: {}", objects(&env.cwd.join("tracker")));
    let times =
        |line: &dyn Fn(usize) -> String| (0..runs).map(|n| time(env, &line(n))).collect::<Vec<_>>();
    report("ready", &times(&|_| "ready".to_owned()));
    report("show", &times(&|_| format!("show {item}")));
    report(
        "edit one field",
        &times(&|n| format!("edit {item} --notes {}-{n} --as-user", label.replace(' ', "-"))),
    );
}

fn run(env: &Env, line: &str) {
    let (code, out) =
        execute(std::iter::once("acyclic").chain(line.split(' ')).map(Into::into), env);
    assert_eq!(code, 0, "acyclic {line}: {out}");
}

fn time(env: &Env, line: &str) -> Duration {
    let start = Instant::now();
    run(env, line);
    start.elapsed()
}

fn report(label: &str, times: &[Duration]) {
    let mut sorted = times.to_vec();
    sorted.sort();
    let median = sorted[sorted.len() / 2];
    println!("{label:>26}: {:>9.1} ms", median.as_secs_f64() * 1000.0);
}

fn gc(repo: &Path) {
    let status = Command::new("git").arg("-C").arg(repo).args(["gc", "--quiet"]).status();
    assert!(status.is_ok_and(|s| s.success()), "git gc");
}

fn objects(repo: &Path) -> String {
    let out = Command::new("git").arg("-C").arg(repo).args(["count-objects", "-v"]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let field = |name: &str| {
        text.lines().find_map(|l| l.strip_prefix(name)).map_or("?", str::trim).to_owned()
    };
    format!("{} loose objects, {} packs", field("count:"), field("packs:"))
}
