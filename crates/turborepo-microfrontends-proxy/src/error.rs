#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("Failed to bind to port {port}: {source}")]
    BindError { port: u16, source: std::io::Error },

    #[error("Hyper error: {0}")]
    Hyper(#[from] hyper::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] hyper::http::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Failed to connect to application '{app}' on port {port}")]
    AppUnreachable { app: String, port: u16 },

    #[error("Invalid request: {0}")]
    InvalidRequest(String),
}

pub struct ErrorPage {
    path: String,
    app: String,
    port: u16,
}

impl ErrorPage {
    pub fn new(path: String, app: String, port: u16) -> Self {
        Self { path, app, port }
    }

    pub fn to_html(&self) -> String {
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Microfrontend Proxy Error</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600;700&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: 'Geist', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: hsl(0, 0%, 100%);
            color: hsl(0, 0%, 9%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            background: hsl(0, 0%, 95%);
            border: 1px solid hsl(0, 0%, 92%);
            border-radius: 12px;
            box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
            max-width: 600px;
            width: 100%;
            padding: 40px;
        }}
        h1 {{
            color: hsl(358, 75%, 59%);
            font-size: 24px;
            margin-bottom: 16px;
            display: flex;
            align-items: center;
            gap: 12px;
        }}
        .error-icon {{
            font-size: 24px;
            flex-shrink: 0;
        }}
        .info-box {{
            background: hsl(0, 0%, 100%);
            border-left: 4px solid hsl(212, 100%, 48%);
            padding: 16px;
            margin: 20px 0;
            border-radius: 4px;
        }}
        .info-box strong {{
            color: hsl(0, 0%, 9%);
            display: block;
            margin-bottom: 8px;
        }}
        .info-box code {{
            background: hsl(0, 0%, 92%);
            padding: 2px 6px;
            border-radius: 3px;
            font-family: 'Geist Mono', 'Monaco', 'Menlo', 'Consolas', monospace;
            font-size: 14px;
            color: hsl(0, 0%, 9%);
        }}
        .details {{
            color: hsl(0, 0%, 40%);
            font-size: 14px;
            line-height: 1.6;
            margin-top: 20px;
        }}
        .troubleshooting {{
            margin-top: 24px;
            padding-top: 24px;
            border-top: 1px solid hsl(0, 0%, 92%);
        }}
        .troubleshooting h2 {{
            font-size: 18px;
            color: hsl(0, 0%, 9%);
            margin-bottom: 12px;
        }}
        .troubleshooting ul {{
            list-style: none;
            padding-left: 0;
        }}
        .troubleshooting li {{
            padding: 8px 0;
            color: hsl(0, 0%, 40%);
            font-size: 14px;
        }}
        .troubleshooting li:before {{
            content: "→";
            color: hsl(212, 100%, 48%);
            font-weight: bold;
            margin-right: 8px;
        }}
        .docs-link {{
            margin-top: 20px;
            padding-top: 20px;
            border-top: 1px solid hsl(0, 0%, 92%);
            font-size: 14px;
            color: hsl(0, 0%, 40%);
        }}
        .docs-link a {{
            color: hsl(212, 100%, 48%);
            text-decoration: none;
        }}
        .docs-link a:hover {{
            text-decoration: underline;
        }}
        @media (prefers-color-scheme: dark) {{
            body {{
                background: hsl(0, 0%, 3.9%);
                color: hsl(0, 0%, 93%);
            }}
            .container {{
                background: hsl(0, 0%, 10%);
                border-color: hsl(0, 0%, 12%);
                box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
            }}
            h1 {{
                color: hsl(358, 100%, 69%);
            }}
            .info-box {{
                background: hsl(0, 0%, 12%);
                border-left-color: hsl(210, 100%, 66%);
            }}
            .info-box strong {{
                color: hsl(0, 0%, 93%);
            }}
            .info-box code {{
                background: hsl(0, 0%, 16%);
                color: hsl(0, 0%, 93%);
            }}
            .details {{
                color: hsl(0, 0%, 63%);
            }}
            .troubleshooting {{
                border-top-color: hsl(0, 0%, 12%);
            }}
            .troubleshooting h2 {{
                color: hsl(0, 0%, 93%);
            }}
            .troubleshooting li {{
                color: hsl(0, 0%, 63%);
            }}
            .troubleshooting li:before {{
                color: hsl(210, 100%, 66%);
            }}
            .docs-link {{
                border-top-color: hsl(0, 0%, 12%);
                color: hsl(0, 0%, 63%);
            }}
            .docs-link a {{
                color: hsl(210, 100%, 66%);
            }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1><span class="error-icon">⚠️</span>Application unreachable</h1>

        <p class="details">
            The Turborepo microfrontends proxy tried to forward your request to the <strong>{app}</strong> application,
            but it's not currently running or not responding on port {port}.
        </p>

        <div class="info-box">
            <strong>Request Path:</strong>
            <code>{path}</code>
        </div>

        <div class="info-box">
            <strong>Expected Application:</strong>
            <code>{app}</code> on port <code>{port}</code>
        </div>

        <div class="troubleshooting">
            <h2>Troubleshooting</h2>
            <ul>
                <li>Make sure the application is running with <code>turbo run dev</code></li>
                <li>Check that port {port} is not being used by another process</li>
                <li>Verify the application configuration in <code>microfrontends.json</code></li>
                <li>Look for errors in the application's console output</li>
            </ul>
            <p class="docs-link">
                For more troubleshooting information, visit <a href="https://turborepo.com/docs/guides/microfrontends" target="_blank">the microfrontends documentation</a>.
            </p>
        </div>
    </div>
</body>
</html>"#,
            path = html_escape(&self.path),
            app = html_escape(&self.app),
            port = self.port,
        )
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_page_html_generation() {
        let page = ErrorPage::new("/docs/api".to_string(), "docs".to_string(), 3001);

        let html = page.to_html();

        assert!(html.contains("/docs/api"));
        assert!(html.contains("docs"));
        assert!(html.contains("3001"));
        assert!(html.contains("Application unreachable"));
        assert!(html.contains("turbo run dev"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(
            html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"
        );
        assert_eq!(html_escape("normal text"), "normal text");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }
}
