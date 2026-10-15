//! Times the `acyclic` binary against trackers of several sizes, one process per command as an agent runs it: setting
//! up a tracker, filling it (single adds below 50 items, an import of a synthetic plan from 50), then medians of
//! reads, writes and a read just after a write, with the store's size on disk.
//!
//! `mise run bench:store -- [--sizes 0,1,10,100,1000,10000] [--runs 5] [--batches 0]` times the binary as shipped;
//! `mise run bench:store:phases -- …` builds it with timing marks and splits each command's time into phases.
//! `--batches` adds that many one-field edits after the first measurements, then times edits and reads again.
//!
//! A claim takes a different ready leaf each run; claiming again renews. "show after edit" is the first read of
//! files the edit just wrote, which an on-access scanner may inspect; repeated reads hide that cost.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use acyclic_core::import::State;
use acyclic_core::probe::PREFIX;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// Whether the binary carries timing marks.
const PHASES: bool = cfg!(feature = "probe");
const AGENT: &str = "session/bench";

fn main() {
    let mut sizes = vec![0, 1, 10, 100, 1_000, 10_000];
    let mut runs = 5;
    let mut batches = 0;
    // `cargo bench` passes `--bench` first.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    for pair in args.chunks(2) {
        match (pair[0].as_str(), pair.get(1)) {
            ("--sizes", Some(list)) => {
                sizes = list.split(',').map(|n| n.parse().expect("a size")).collect();
            },
            ("--runs", Some(n)) => runs = n.parse().expect("a run count"),
            ("--batches", Some(n)) => batches = n.parse().expect("a batch count"),
            _ => {
                eprintln!("usage: store [--sizes <n,n,…>] [--runs <n>] [--batches <n>]");
                std::process::exit(2);
            },
        }
    }
    let started = Instant::now();
    let mut table = Vec::new();
    for &size in &sizes {
        table.push(bench(size, runs, batches));
    }
    println!("\nmedians in ms of {runs} runs; {PHASES_NOTE}", PHASES_NOTE = phases_note());
    print_table(&table, batches);
    println!("\nsweep took {:.1} s", started.elapsed().as_secs_f64());
}

fn phases_note() -> &'static str {
    if PHASES { "with timing marks, which add a little" } else { "the binary as shipped" }
}

/// One command's wall time and, with timing marks, its phases.
struct Sample {
    wall: Duration,
    phases: Vec<(String, Duration)>,
}

/// A tracker in its own directory, driven through the binary.
struct Tracker {
    dir: TempDir,
}

impl Tracker {
    fn new() -> Self {
        Self { dir: TempDir::new().expect("a temporary directory") }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn repo(&self) -> std::path::PathBuf {
        self.path().join("tracker")
    }

    /// Runs `acyclic <line>` as the user, or as an agent, and returns its output and timing.
    fn run(&self, line: &str, agent: bool) -> (serde_json::Value, Sample) {
        let mut command = Command::new(BIN);
        command
            .args(line.split(' '))
            .current_dir(self.path())
            .env_clear()
            .env("HOME", self.path())
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("ACYCLIC_TRACKER", "tracker")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        if agent {
            command.env("ACYCLIC_ACTOR", AGENT);
        } else if !line.starts_with("init") && is_write(line) {
            command.arg("--as-user");
        }
        let start = Instant::now();
        let output = command.output().expect("the binary runs");
        let wall = start.elapsed();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "acyclic {line}: {stdout} {stderr}");
        let mut phases = Vec::new();
        for mark in stderr.lines().filter_map(|l| l.strip_prefix(PREFIX)?.strip_prefix('\t')) {
            let (name, micros) = mark.rsplit_once('\t').expect("a mark has a time");
            phases.push((name.to_owned(), Duration::from_micros(micros.parse().expect("micros"))));
        }
        assert_eq!(PHASES, !phases.is_empty(), "marks appear exactly when the binary carries them");
        (serde_json::from_str(&stdout).unwrap_or_default(), Sample { wall, phases })
    }

    fn time(&self, line: &str, agent: bool) -> Sample {
        self.run(line, agent).1
    }
}

fn is_write(line: &str) -> bool {
    !["ready", "show", "needs", "doctor"].iter().any(|read| line.starts_with(read))
}

/// One size's results: each operation's median, in milliseconds, and the store's size.
#[derive(Default)]
struct Row {
    size: usize,
    times: BTreeMap<&'static str, f64>,
    store: String,
}

fn bench(size: usize, runs: usize, batches: usize) -> Row {
    println!("\n=== {size} items");
    let mut row = Row { size, ..Row::default() };
    let record = |row: &mut Row, label: &'static str, samples: &[Sample]| {
        row.times.insert(label, report(label, samples));
    };
    let inits: Vec<Sample> =
        (0..runs).map(|_| Tracker::new().time("init tracker --replica bench", false)).collect();
    record(&mut row, "init", &inits);
    let tracker = Tracker::new();
    tracker.run("init tracker --replica bench", false);
    if size == 0 {
        record(
            &mut row,
            "ready",
            &(0..runs).map(|_| tracker.time("ready", false)).collect::<Vec<_>>(),
        );
        let adds: Vec<Sample> =
            (0..runs).map(|n| tracker.time(&format!("add z{n} --title First"), false)).collect();
        record(&mut row, "edit", &adds[..1]);
        println!("  (the edit column is the first add)");
        row.store = store_size(&tracker.repo());
        return row;
    }
    let item = if size < 50 {
        let adds: Vec<Sample> =
            (0..size).map(|n| tracker.time(&format!("add i{n} --title Item-{n}"), false)).collect();
        report("add", &adds);
        format!("i{}", size - 1)
    } else {
        let plan = acyclic_fixtures::generate(size, 7);
        std::fs::write(tracker.path().join("plan.json"), plan.to_json()).expect("the plan");
        record(&mut row, "import", &[tracker.time("import plan.json", false)]);
        record(&mut row, "import again", &[tracker.time("import plan.json", false)]);
        let open = plan.items.iter().rev().find(|i| i.state != State::Done);
        open.expect("an open item").key.clone()
    };
    tracker.run("add bench-target --title Target", false);
    row.store = store_size(&tracker.repo());
    println!("  store: {}", row.store);

    let times = |line: &dyn Fn(usize) -> String, agent| {
        (0..runs).map(|n| tracker.time(&line(n), agent)).collect::<Vec<_>>()
    };
    record(&mut row, "ready", &times(&|_| "ready".into(), false));
    let cache = tracker.repo().join(".git/acyclic/cache");
    let cold: Vec<Sample> = (0..runs)
        .map(|_| {
            let _ = std::fs::remove_dir_all(&cache);
            tracker.time("ready", false)
        })
        .collect();
    record(&mut row, "ready, no cache", &cold);
    record(&mut row, "show", &times(&|_| format!("show {item}"), false));
    record(&mut row, "edit", &times(&|n| format!("edit {item} --notes edit-{n}"), false));
    let fresh: Vec<Sample> = (0..runs)
        .map(|n| {
            tracker.run(&format!("edit {item} --notes fresh-{n}"), false);
            tracker.time(&format!("show {item}"), false)
        })
        .collect();
    record(&mut row, "show after edit", &fresh);
    let (ready, _) = tracker.run("ready", false);
    let leaves: Vec<String> = ready["data"]["ready"]
        .as_array()
        .expect("a ready list")
        .iter()
        .take(runs)
        .map(|leaf| leaf["item"].as_str().expect("an item id").to_owned())
        .collect();
    let claims: Vec<Sample> =
        leaves.iter().map(|leaf| tracker.time(&format!("claim {leaf}"), true)).collect();
    record(&mut row, "claim", &claims);
    record(&mut row, "claim again", &times(&|_| format!("claim {}", leaves[0]), true));
    let links: Vec<Sample> = (0..runs)
        .map(|_| {
            let sample = tracker.time(&format!("link {item} blocks bench-target"), false);
            tracker.run(&format!("unlink {item} blocks bench-target"), false);
            sample
        })
        .collect();
    record(&mut row, "link", &links);
    if batches > 0 {
        // Distinct items, so the cache's delta grows as an agent's day of edits would grow it.
        let (ready, _) = tracker.run("ready", false);
        let mut items: Vec<String> = ready["data"]["ready"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|leaf| leaf["item"].as_str().map(str::to_owned))
            .collect();
        items.push(item.clone());
        for n in 0..batches {
            tracker.run(&format!("edit {} --notes batch-{n}", items[n % items.len()]), false);
        }
        println!("  after {batches} more batches: {}", store_size(&tracker.repo()));
        record(
            &mut row,
            "edit after batches",
            &times(&|n| format!("edit {item} --notes late-{n}"), false),
        );
        record(&mut row, "show after batches", &times(&|_| format!("show {item}"), false));
    }
    row
}

/// Prints an operation's median and, with timing marks, the median of each phase; returns the median in ms.
fn report(label: &str, samples: &[Sample]) -> f64 {
    let wall = median(samples.iter().map(|s| s.wall).collect());
    println!("  {label:<24} {:>9.1} ms", ms(wall));
    if PHASES {
        let mut names: Vec<&str> = Vec::new();
        for (name, _) in samples.iter().flat_map(|s| &s.phases) {
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
        let mut accounted = Duration::ZERO;
        for name in names {
            let phase = median(
                samples
                    .iter()
                    .map(|s| s.phases.iter().filter(|(n, _)| n == name).map(|(_, d)| *d).sum())
                    .collect(),
            );
            accounted += phase;
            if phase >= Duration::from_micros(500) {
                println!("      {name:<34} {:>9.1}", ms(phase));
            }
        }
        println!(
            "      {:<34} {:>9.1}",
            "unmarked: process start and exit",
            ms(wall.saturating_sub(accounted))
        );
    }
    ms(wall)
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// The object store's and the cache's bytes on disk, and the pack count.
fn store_size(repo: &Path) -> String {
    let git = repo.join(".git");
    let packs = std::fs::read_dir(git.join("objects/pack")).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "pack"))
            .count()
    });
    format!(
        "objects {:.1} MiB in {packs} packs, cache {:.1} MiB",
        mib(du(&git.join("objects"))),
        mib(du(&git.join("acyclic/cache")))
    )
}

fn du(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => du(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

#[allow(clippy::cast_precision_loss)]
fn mib(bytes: u64) -> f64 {
    bytes as f64 / f64::from(1 << 20)
}

fn print_table(rows: &[Row], batches: usize) {
    let mut columns = vec![
        "init",
        "import",
        "import again",
        "ready",
        "ready, no cache",
        "show",
        "show after edit",
        "edit",
        "claim",
        "claim again",
        "link",
    ];
    if batches > 0 {
        columns.extend(["edit after batches", "show after batches"]);
    }
    println!("\n| items | {} | store |", columns.join(" | "));
    println!("|---:|{}---|", "---:|".repeat(columns.len()));
    for row in rows {
        let cells: Vec<String> = columns
            .iter()
            .map(|column| row.times.get(column).map_or("—".into(), |t| format!("{t:.0}")))
            .collect();
        println!("| {} | {} | {} |", row.size, cells.join(" | "), row.store);
    }
}
