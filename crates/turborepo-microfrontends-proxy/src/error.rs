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
}

pub struct ErrorPage {
    path: String,
    app: String,
    port: u16,
    error_message: String,
}

impl ErrorPage {
    pub fn new(path: String, app: String, port: u16, error_message: String) -> Self {
        Self {
            path,
            app,
            port,
            error_message,
        }
    }

    pub fn to_html(&self) -> String {
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Microfrontend Proxy Error</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            background: white;
            border-radius: 12px;
            box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
            max-width: 600px;
            width: 100%;
            padding: 40px;
        }}
        h1 {{
            color: #e53e3e;
            font-size: 24px;
            margin-bottom: 16px;
        }}
        .error-icon {{
            width: 64px;
            height: 64px;
            background: #fed7d7;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            margin: 0 auto 24px;
            font-size: 32px;
        }}
        .info-box {{
            background: #f7fafc;
            border-left: 4px solid #4299e1;
            padding: 16px;
            margin: 20px 0;
            border-radius: 4px;
        }}
        .info-box strong {{
            color: #2d3748;
            display: block;
            margin-bottom: 8px;
        }}
        .info-box code {{
            background: #edf2f7;
            padding: 2px 6px;
            border-radius: 3px;
            font-family: 'Monaco', 'Menlo', 'Consolas', monospace;
            font-size: 14px;
            color: #2d3748;
        }}
        .command {{
            background: #2d3748;
            color: #f7fafc;
            padding: 16px;
            border-radius: 6px;
            font-family: 'Monaco', 'Menlo', 'Consolas', monospace;
            font-size: 14px;
            margin: 20px 0;
            overflow-x: auto;
        }}
        .details {{
            color: #718096;
            font-size: 14px;
            line-height: 1.6;
            margin-top: 20px;
        }}
        .troubleshooting {{
            margin-top: 24px;
            padding-top: 24px;
            border-top: 1px solid #e2e8f0;
        }}
        .troubleshooting h2 {{
            font-size: 18px;
            color: #2d3748;
            margin-bottom: 12px;
        }}
        .troubleshooting ul {{
            list-style: none;
            padding-left: 0;
        }}
        .troubleshooting li {{
            padding: 8px 0;
            color: #4a5568;
            font-size: 14px;
        }}
        .troubleshooting li:before {{
            content: "→";
            color: #4299e1;
            font-weight: bold;
            margin-right: 8px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-icon">⚠️</div>
        <h1>Application Not Reachable</h1>

        <div class="info-box">
            <strong>Request Path:</strong>
            <code>{path}</code>
        </div>

        <div class="info-box">
            <strong>Expected Application:</strong>
            <code>{app}</code> on port <code>{port}</code>
        </div>

        <div class="info-box">
            <strong>Error:</strong>
            <code>{error}</code>
        </div>

        <p class="details">
            The Turborepo microfrontends proxy tried to forward your request to the <strong>{app}</strong> application,
            but it's not currently running or not responding on port {port}.
        </p>

        <div class="command">
turbo run {app}#dev
        </div>

        <div class="troubleshooting">
            <h2>Troubleshooting</h2>
            <ul>
                <li>Make sure the application is running with <code>turbo dev</code></li>
                <li>Check that port {port} is not being used by another process</li>
                <li>Verify the application configuration in <code>microfrontends.json</code></li>
                <li>Look for errors in the application's console output</li>
            </ul>
        </div>
    </div>
</body>
</html>"#,
            path = html_escape(&self.path),
            app = html_escape(&self.app),
            port = self.port,
            error = html_escape(&self.error_message),
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
        let page = ErrorPage::new(
            "/docs/api".to_string(),
            "docs".to_string(),
            3001,
            "Connection refused".to_string(),
        );

        let html = page.to_html();

        assert!(html.contains("/docs/api"));
        assert!(html.contains("docs"));
        assert!(html.contains("3001"));
        assert!(html.contains("Connection refused"));
        assert!(html.contains("turbo run docs#dev"));
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
