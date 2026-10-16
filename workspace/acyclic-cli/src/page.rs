//! The graph as a self-contained HTML page: the document, a renderer and a little pan and zoom, with no network.

/// The renderers fetched at build time, `None` when the binary was built without them.
mod renderers {
    include!(concat!(env!("OUT_DIR"), "/renderers.rs"));
}

const TEMPLATE: &str = include_str!("../assets/page.html");

/// What lays the graph out in the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Renderer {
    /// Graphviz, from a DOT document.
    Graphviz,
    /// Mermaid, from a flowchart.
    Mermaid,
}

/// The page for `source`, a DOT document for Graphviz or a flowchart for Mermaid, or `None` when this binary was
/// built without that renderer.
pub fn page(title: &str, renderer: Renderer, source: &str) -> Option<String> {
    let (name, library) = match renderer {
        // An inline module, since browsers differ on importing one from a Blob URL in a file's page.
        Renderer::Graphviz => (
            "graphviz",
            format!(
                "<script type=\"module\">{}\nglobalThis.acyclicGraphviz = {};</script>",
                renderers::GRAPHVIZ?,
                renderers::GRAPHVIZ_BINDING?
            ),
        ),
        Renderer::Mermaid => ("mermaid", format!("<script>{}</script>", renderers::MERMAID?)),
    };
    // The source holds titles from any actor: as a JSON string with `<`, `>` and `&` escaped, it can't end its
    // script element or start markup.
    let source = serde_json::to_string(source)
        .expect("a string serializes")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let mut out = String::with_capacity(TEMPLATE.len() + library.len() + source.len());
    let mut rest = TEMPLATE;
    for (marker, value) in [
        ("<!--TITLE-->", escape(title)),
        ("<!--RENDERER-->", name.to_owned()),
        ("<!--SOURCE-->", source),
        ("<!--LIBRARY-->", library),
    ] {
        let (before, after) =
            rest.split_once(marker).expect("the template holds each marker once, in order");
        out.push_str(before);
        out.push_str(&value);
        rest = after;
    }
    out.push_str(rest);
    Some(out)
}

/// Text as HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_names_every_pin_the_fetch_checks() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let script = std::fs::read_to_string(format!("{root}/scripts/vendor-graph.pins")).unwrap();
        let notice = std::fs::read_to_string(format!("{root}/THIRD_PARTY.md")).unwrap();
        let pins = script.lines().filter(|line| !line.starts_with('#'));
        let fields: Vec<&str> = pins.flat_map(str::split_whitespace).collect();
        assert_eq!(fields.len(), 10, "two renderers of five fields");
        for field in
            fields.iter().filter(|field| field.starts_with("https://") || field.len() == 64)
        {
            assert!(notice.contains(field), "THIRD_PARTY.md lacks {field}");
        }
    }

    #[test]
    fn untrusted_source_stays_inside_its_script_element() {
        let hostile = "</script><script>window.pwned = 1</script> & <!-- \u{2028}";
        for renderer in [Renderer::Graphviz, Renderer::Mermaid] {
            let Some(html) = page("a </title> & b", renderer, hostile) else { continue };
            let start = html.find("<script id=\"source\" type=\"application/json\">").unwrap();
            let end = start + html[start..].find("</script>").unwrap();
            let json = &html[start..end].split_once('>').unwrap().1;
            assert_eq!(serde_json::from_str::<String>(json).unwrap(), hostile);
            assert!(!json.contains('<') && !json.contains('>') && !json.contains('&'));
            assert!(html.contains("<title>a &lt;/title&gt; &amp; b</title>"));
        }
    }
}
