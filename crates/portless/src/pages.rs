//! HTML error pages used by the local proxy.

/// The arrow displayed beside an active route.
pub const ARROW_SVG: &str = r#"<svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="M6.5 3.5L11 8l-4.5 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

const PAGE_STYLES: &str = r#"
  *, *::before, *::after { margin: 0; padding: 0; box-sizing: border-box; }
  :root {
    --bg: #fff; --fg: #171717; --border: #eaeaea; --surface: #fafafa;
    --text-2: #666; --text-3: #a1a1a1; --accent: #0070f3;
    --font-sans: 'Geist', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
    --font-mono: 'Geist Mono', 'SFMono-Regular', Menlo, Monaco, Consolas, monospace;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #000; --fg: #ededed; --border: rgba(255,255,255,0.1);
      --surface: #111; --text-2: #888; --text-3: #666; --accent: #3291ff;
    }
  }
  html { height: 100%; }
  body {
    font-family: var(--font-sans); background: var(--bg); color: var(--fg);
    min-height: 100%; -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }
  .page {
    min-height: 100vh; display: flex; flex-direction: column; align-items: center;
    justify-content: center; padding: 32px 24px;
  }
  .hero { display: flex; flex-direction: column; align-items: center; }
  .hero h1 {
    font-family: 'Geist Pixel', var(--font-mono); font-size: clamp(80px, 15vw, 144px);
    font-weight: 400; line-height: 1; letter-spacing: -0.04em; color: var(--fg);
  }
  .hero h2 {
    font-size: 13px; font-weight: 400; color: var(--text-3); margin-top: 16px;
    text-transform: uppercase; letter-spacing: 0.15em;
  }
  .content { margin-top: 56px; width: 100%; max-width: 480px; }
  .desc { font-size: 14px; color: var(--text-2); text-align: center; line-height: 1.7; }
  .desc strong { color: var(--fg); font-weight: 500; }
  .section { margin-top: 32px; }
  .label {
    font-size: 12px; font-weight: 500; color: var(--text-3); text-transform: uppercase;
    letter-spacing: 0.1em; margin-bottom: 10px;
  }
  .card { list-style: none; border: 1px solid var(--border); border-radius: 12px; overflow: hidden; }
  .card > li { border-bottom: 1px solid var(--border); }
  .card > li:last-child { border-bottom: none; }
  .card-link {
    display: flex; align-items: center; justify-content: space-between; padding: 14px 16px;
    text-decoration: none; color: inherit; transition: background 0.15s ease;
  }
  .card-link:hover { background: var(--surface); }
  .card-link .name { font-size: 14px; font-weight: 500; transition: color 0.15s ease; }
  .card-link:hover .name { color: var(--accent); }
  .card-link .meta { display: flex; align-items: center; gap: 10px; }
  .card-link .port { font-family: var(--font-mono); font-size: 13px; color: var(--text-3); }
  .card-link .arrow { color: var(--text-3); display: flex; transition: transform 0.2s ease, color 0.2s ease; }
  .card-link:hover .arrow { transform: translateX(2px); color: var(--text-2); }
  .terminal {
    font-family: var(--font-mono); font-size: 13px; background: var(--surface);
    border: 1px solid var(--border); border-radius: 12px; padding: 14px 20px;
    line-height: 1.7; color: var(--fg);
  }
  .terminal .prompt { color: var(--text-3); user-select: none; }
  pre.terminal { white-space: pre-wrap; }
  .empty { font-size: 14px; color: var(--text-3); text-align: center; padding: 32px 0; }
  .footer {
    margin-top: 64px; font-size: 11px; color: var(--text-3);
    font-family: var(--font-mono); letter-spacing: 0.08em;
  }
"#;

/// Render a complete Portless status page.
#[must_use]
pub fn render_page(status: u16, status_text: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{status} - {status_text}</title>
<style>{PAGE_STYLES}</style>
</head>
<body>
<div class="page">
<div class="hero"><h1>{status}</h1><h2>{status_text}</h2></div>
{body}
<p class="footer">portless</p>
</div>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::{render_page, ARROW_SVG};

    #[test]
    fn renders_complete_accessible_document() {
        let page = render_page(404, "Not Found", r#"<p class="desc">missing</p>"#);
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(page.contains("<title>404 - Not Found</title>"));
        assert!(page.contains(r#"<meta name="color-scheme" content="light dark">"#));
        assert!(page.contains(r#"<div class="hero"><h1>404</h1><h2>Not Found</h2></div>"#));
        assert!(page.contains(r#"<p class="desc">missing</p>"#));
        assert!(page.ends_with("</html>"));
    }

    #[test]
    fn arrow_matches_the_portless_asset() {
        assert!(ARROW_SVG.contains(r#"viewBox="0 0 16 16""#));
        assert!(ARROW_SVG.contains("M6.5 3.5L11 8l-4.5 4.5"));
    }
}
