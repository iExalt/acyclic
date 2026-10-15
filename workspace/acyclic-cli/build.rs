//! The graph page embeds two renderers that `mise run vendor:graph` fetches and nobody commits. Without them the binary
//! still builds, and only the HTML page refuses; then cargo reruns this script on every build, since the files it
//! watches are missing. `ACYCLIC_REQUIRE_ASSETS=1` (the checks, CI and releases) demands them, each matching its pin in
//! `scripts/vendor-graph.pins`, so such a binary embeds exactly the pinned bytes.
//!
//! Each file must also be safe to inline in a page: it may not end its script element early, or enter the parser's
//! escaped state, which would hide the element's end. Graphviz's module exports its class under a minified name, which
//! the page needs.

use std::fmt::Write as _;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo::rerun-if-env-changed=ACYCLIC_REQUIRE_ASSETS");
    let required = std::env::var("ACYCLIC_REQUIRE_ASSETS").is_ok_and(|value| value == "1");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let pins_path = manifest.join("../../scripts/vendor-graph.pins");
    println!("cargo::rerun-if-changed={}", pins_path.display());
    let pins = std::fs::read_to_string(&pins_path).expect("the pins are committed");
    let mut code = String::new();
    for line in pins.lines().filter(|line| !line.starts_with('#') && !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [name, _, _, _, digest] = fields[..] else { panic!("a pin has five fields: {line}") };
        let path = manifest.join("assets/graph").join(format!("{name}.js"));
        println!("cargo::rerun-if-changed={}", path.display());
        let value = match std::fs::read(&path) {
            Ok(bytes) => {
                let actual = Sha256::digest(&bytes).iter().fold(String::new(), |mut hex, byte| {
                    write!(hex, "{byte:02x}").expect("a String takes writes");
                    hex
                });
                assert!(
                    !required || actual == digest,
                    "{} doesn't match its pin: run `mise run vendor:graph`",
                    path.display()
                );
                let text = String::from_utf8(bytes).expect("a renderer is UTF-8");
                let lower = text.to_ascii_lowercase();
                assert!(
                    !lower.contains("</script"),
                    "{} would end its script element",
                    path.display()
                );
                assert!(
                    !(lower.contains("<!--") && lower.contains("<script")),
                    "{} could hide its script element's end",
                    path.display()
                );
                format!("Some(include_str!({:?}))", path.display().to_string())
            },
            Err(_) if required => panic!(
                "{} is missing: run `mise run vendor:graph` to fetch the graph page's renderers",
                path.display()
            ),
            Err(_) => "None".to_owned(),
        };
        writeln!(code, "pub const {}: Option<&str> = {value};", name.to_ascii_uppercase())
            .expect("a String takes writes");
        if name == "graphviz" {
            let binding = std::fs::read_to_string(&path).ok().map(|text| graphviz_binding(&text));
            writeln!(code, "pub const GRAPHVIZ_BINDING: Option<&str> = {binding:?};")
                .expect("a String takes writes");
        }
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("renderers.rs");
    std::fs::write(out, code).expect("OUT_DIR is writable");
}

/// The local name the module's last `export{…}` gives as `Graphviz`.
fn graphviz_binding(text: &str) -> String {
    let start = text.rfind("export{").expect("the Graphviz module ends with an export list")
        + "export{".len();
    let list = &text[start..start + text[start..].find('}').expect("the export list closes")];
    list.split(',')
        .find_map(|entry| entry.trim().strip_suffix(" as Graphviz"))
        .filter(|name| name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$'))
        .expect("the module exports Graphviz under a plain name")
        .to_owned()
}
