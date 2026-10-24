//! Embeds every file under the app's `dist`, with the `.br` and `.gz` copies the build wrote beside each one, as a table
//! sorted by path. Without `dist/index.html` the table holds a stub page instead, unless `ACYCLIC_REQUIRE_APP=1`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-env-changed=ACYCLIC_REQUIRE_APP");
    let required = std::env::var("ACYCLIC_REQUIRE_APP").is_ok_and(|value| value == "1");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let dist = manifest.join("../acyclic-app/dist");
    // Watching the directory reruns this script when anything under it changes, or when it appears.
    println!("cargo::rerun-if-changed={}", dist.display());
    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        walk(&dist, &dist, &mut files);
    } else if required {
        panic!(
            "{} is missing: run `mise run build:app` to build the app",
            dist.join("index.html").display()
        );
    } else {
        println!(
            "cargo::warning=the app isn't built, so the server serves a stub page: run `mise run build:app`"
        );
    }
    files.sort();
    let mut code = format!(
        "/// Whether the app's build is embedded, rather than the stub page.\npub const BUILT: bool = {};\n",
        !files.is_empty()
    );
    code.push_str("/// Every file, sorted by path.\npub static FILES: &[Asset] = &[\n");
    if files.is_empty() {
        code.push_str("    Asset { path: \"/index.html\", body: STUB, br: None, gz: None },\n");
    }
    for (path, file) in &files {
        let variant = |suffix: &str| {
            let compressed = file.with_extension(format!(
                "{}.{suffix}",
                file.extension().and_then(|ext| ext.to_str()).unwrap_or_default()
            ));
            if compressed.is_file() {
                format!("Some(include_bytes!({:?}))", compressed.display().to_string())
            } else {
                "None".to_owned()
            }
        };
        writeln!(
            code,
            "    Asset {{ path: {path:?}, body: include_bytes!({:?}), br: {}, gz: {} }},",
            file.display().to_string(),
            variant("br"),
            variant("gz")
        )
        .expect("a String takes writes");
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("files.rs");
    std::fs::write(out, code).expect("OUT_DIR is writable");
}

/// Collects the files under `dir` as `(/path/from/root, file)`, leaving out the compressed copies.
fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("dist is readable") {
        let path = entry.expect("dist is readable").path();
        if path.is_dir() {
            walk(root, &path, files);
        } else if !matches!(path.extension().and_then(|ext| ext.to_str()), Some("br" | "gz")) {
            let relative = path.strip_prefix(root).expect("under the root");
            let parts: Vec<_> =
                relative.components().map(|part| part.as_os_str().to_string_lossy()).collect();
            files.push((format!("/{}", parts.join("/")), path));
        }
    }
}
