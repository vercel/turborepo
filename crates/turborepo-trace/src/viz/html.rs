//! The HTML report: one file, with its style, its script and its data inline.
//! It loads nothing: no network request, no font, no image.

use anyhow::Result;

use super::Model;

const PAGE: &str = include_str!("report.html");
const STYLE: &str = include_str!("report.css");
const SCRIPT: &str = include_str!("report.js");

/// Where the parts go in `report.html`.
const STYLE_SLOT: &str = "/*STYLE*/";
const DATA_SLOT: &str = "/*DATA*/";
const SCRIPT_SLOT: &str = "/*SCRIPT*/";

/// The page for `model`.
pub(super) fn render(model: &Model) -> Result<String> {
    Ok(assemble(&serde_json::to_string(model)?))
}

/// The page with `data` (JSON) embedded. `<` is escaped, so that no string of
/// the data (a path, a detail of an uncertainty record) can end the element it
/// is in.
// Invariant: the embedded template is fixed and has them (unit-tested).
#[allow(clippy::expect_used)]
pub(super) fn assemble(data: &str) -> String {
    let data = data.replace('<', "\\u003c");
    let mut out = String::with_capacity(PAGE.len() + STYLE.len() + SCRIPT.len() + data.len());
    let mut rest = PAGE;
    for (slot, part) in [
        (STYLE_SLOT, STYLE),
        (DATA_SLOT, data.as_str()),
        (SCRIPT_SLOT, SCRIPT),
    ] {
        let (head, tail) = rest
            .split_once(slot)
            .expect("report.html has its three slots, in order");
        out.push_str(head);
        out.push_str(part);
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page loads nothing and cannot be broken out of by its data.
    #[test]
    fn self_contained() {
        let page = assemble(r#"{"nodes":["a</script><script>alert(1)</script>","<!--"]}"#);
        assert_eq!(page.matches("</script>").count(), 2, "data and script");
        let lt = "\\u003c";
        assert!(page.contains(&format!("a{lt}/script>{lt}script>alert(1){lt}/script>")));
        assert!(!page.contains("<!--"));
        for needle in [
            "http://",
            "https://",
            "<script src",
            "<link",
            "@import",
            "<img",
            "<iframe",
            "fetch(",
            "XMLHttpRequest",
            "WebSocket",
            "sendBeacon",
            "EventSource",
            "import(",
        ] {
            assert!(
                !page.contains(needle),
                "the report must not contain {needle}"
            );
        }
        // The only `url()` is a reference to a pattern inside the page.
        assert!(
            page.match_indices("url(")
                .all(|(i, _)| page[i..].starts_with("url(#")),
            "the report must not load a resource"
        );
        assert!(
            !SCRIPT.contains("</script"),
            "the script must not end its element"
        );
        assert!(
            !STYLE.contains("</style"),
            "the style must not end its element"
        );
    }
}
