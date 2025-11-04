use std::{collections::HashMap, sync::Arc};

use turborepo_microfrontends::Config;

use crate::ports::validate_port;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    pub app_name: Arc<str>,
    pub port: u16,
    pub fallback: Option<Arc<str>>,
}

#[derive(Clone)]
pub struct Router {
    trie: TrieNode,
    apps: Vec<AppInfo>,
    default_app_idx: usize,
}

#[derive(Debug, Clone)]
struct AppInfo {
    app_name: Arc<str>,
    port: u16,
    fallback: Option<Arc<str>>,
}

#[derive(Clone, Default)]
struct TrieNode {
    exact_children: HashMap<Arc<str>, TrieNode>,
    param_child: Option<Box<TrieNode>>,
    wildcard_match: Option<usize>,
    terminal_match: Option<usize>,
}

#[derive(Debug, Clone)]
struct Route {
    app_name: String,
    port: u16,
    patterns: Vec<PathPattern>,
}

#[derive(Debug, Clone)]
struct PathPattern {
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Exact(Arc<str>),
    Param,
    Wildcard,
}

impl Router {
    pub fn new(config: &Config) -> Result<Self, String> {
        let mut routes = Vec::new();
        let mut default_app = None;
        let mut app_ports: HashMap<String, u16> = HashMap::new();

        for app in config.applications() {
            let app_name = app.application_name;
            let port = config.port(app_name).ok_or_else(|| {
                format!(
                    "No port configured for application '{app_name}'. Check your configuration \
                     file."
                )
            })?;

            // Validate port for security (SSRF prevention)
            validate_port(port)
                .map_err(|e| format!("Invalid port {port} for application '{app_name}': {e}"))?;

            app_ports.insert(app_name.to_string(), port);

            if let Some(routing) = config.routing(app_name) {
                let mut patterns = Vec::new();
                for path_group in routing {
                    let mut group_patterns = Vec::with_capacity(path_group.paths.len());
                    for path in &path_group.paths {
                        group_patterns.push(PathPattern::parse(path).map_err(|e| {
                            format!(
                                "Invalid routing pattern '{path}' for application '{app_name}': \
                                 {e}"
                            )
                        })?);
                    }
                    patterns.extend(group_patterns);
                }

                routes.push(Route {
                    app_name: app_name.to_string(),
                    port,
                    patterns,
                });
            } else if default_app.is_none() {
                default_app = Some((app_name.to_string(), port));
            }
        }

        let default_app = default_app.ok_or_else(|| {
            "No default application found. At least one application without routing configuration \
             is required."
                .to_string()
        })?;

        let mut apps = Vec::new();
        let mut trie = TrieNode::default();

        for route in routes {
            let app_idx = apps.len();
            let fallback = config
                .fallback(&route.app_name)
                .map(|s| Arc::from(s.to_string()));
            apps.push(AppInfo {
                app_name: Arc::from(route.app_name),
                port: route.port,
                fallback,
            });

            for pattern in route.patterns {
                trie.insert(&pattern.segments, app_idx);
            }
        }

        let default_app_idx = apps.len();
        let default_fallback = config
            .fallback(&default_app.0)
            .map(|s| Arc::from(s.to_string()));
        apps.push(AppInfo {
            app_name: Arc::from(default_app.0),
            port: default_app.1,
            fallback: default_fallback,
        });

        Ok(Self {
            trie,
            apps,
            default_app_idx,
        })
    }

    pub fn match_route(&self, path: &str) -> RouteMatch {
        let path = path.strip_prefix('/').unwrap_or(path);

        let app_idx = if path.is_empty() {
            self.trie.lookup(&[])
        } else {
            let mut segments = Vec::new();
            for segment in path.split('/') {
                if !segment.is_empty() {
                    segments.push(segment);
                }
            }
            self.trie.lookup(&segments)
        }
        .unwrap_or(self.default_app_idx);

        let app = &self.apps[app_idx];
        RouteMatch {
            app_name: app.app_name.clone(),
            port: app.port,
            fallback: app.fallback.clone(),
        }
    }
}

impl TrieNode {
    fn insert(&mut self, segments: &[Segment], app_idx: usize) {
        if segments.is_empty() {
            self.terminal_match = Some(app_idx);
            return;
        }

        match &segments[0] {
            Segment::Exact(name) => {
                let child = self.exact_children.entry(Arc::clone(name)).or_default();
                child.insert(&segments[1..], app_idx);
            }
            Segment::Param => {
                let child = self
                    .param_child
                    .get_or_insert_with(|| Box::new(TrieNode::default()));
                child.insert(&segments[1..], app_idx);
            }
            Segment::Wildcard => {
                self.wildcard_match = Some(app_idx);
            }
        }
    }

    fn lookup(&self, segments: &[&str]) -> Option<usize> {
        if segments.is_empty() {
            return self.terminal_match.or(self.wildcard_match);
        }

        if let Some(child) = self.exact_children.get(segments[0])
            && let Some(app_idx) = child.lookup(&segments[1..])
        {
            return Some(app_idx);
        }

        if let Some(child) = &self.param_child
            && let Some(app_idx) = child.lookup(&segments[1..])
        {
            return Some(app_idx);
        }

        if let Some(app_idx) = self.wildcard_match {
            return Some(app_idx);
        }

        None
    }
}

impl PathPattern {
    fn parse(pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err(
                "Routing pattern cannot be empty. Provide a valid path pattern like '/' or \
                 '/docs/:path*'"
                    .to_string(),
            );
        }

        let pattern = pattern.strip_prefix('/').unwrap_or(pattern);

        if pattern.is_empty() {
            return Ok(Self { segments: vec![] });
        }

        let mut segments = Vec::new();
        for segment in pattern.split('/') {
            if segment.is_empty() {
                continue;
            }

            if let Some(param_name) = segment.strip_prefix(':') {
                if param_name.is_empty() {
                    return Err(
                        "Parameter name cannot be empty after ':'. Use a format like ':id' or \
                         ':path*'"
                            .to_string(),
                    );
                }
                if param_name.ends_with('*') {
                    segments.push(Segment::Wildcard);
                } else {
                    segments.push(Segment::Param);
                }
            } else {
                segments.push(Segment::Exact(Arc::from(segment)));
            }
        }

        Ok(Self { segments })
    }

    #[cfg(test)]
    fn matches(&self, path: &str) -> bool {
        let path = if let Some(stripped) = path.strip_prefix('/') {
            stripped
        } else {
            path
        };

        if path.is_empty() && self.segments.is_empty() {
            return true;
        }

        let path_segments: Vec<&str> = if path.is_empty() {
            vec![]
        } else {
            path.split('/').collect()
        };

        self.matches_segments(&path_segments)
    }

    #[cfg(test)]
    fn matches_segments(&self, path_segments: &[&str]) -> bool {
        let mut pattern_idx = 0;
        let mut path_idx = 0;

        while pattern_idx < self.segments.len() && path_idx < path_segments.len() {
            match &self.segments[pattern_idx] {
                Segment::Exact(expected) => {
                    if path_segments[path_idx] != expected.as_ref() {
                        return false;
                    }
                    pattern_idx += 1;
                    path_idx += 1;
                }
                Segment::Param => {
                    pattern_idx += 1;
                    path_idx += 1;
                }
                Segment::Wildcard => {
                    return true;
                }
            }
        }

        if pattern_idx < self.segments.len() {
            matches!(self.segments[pattern_idx], Segment::Wildcard)
        } else {
            path_idx == path_segments.len()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let pattern = PathPattern::parse("/blog").unwrap();
        assert!(pattern.matches("/blog"));
        assert!(!pattern.matches("/blog/post"));
        assert!(!pattern.matches("/blogs"));
        assert!(!pattern.matches("/"));
    }

    #[test]
    fn test_param_match() {
        let pattern = PathPattern::parse("/blog/:slug").unwrap();
        assert!(pattern.matches("/blog/hello"));
        assert!(pattern.matches("/blog/world"));
        assert!(!pattern.matches("/blog"));
        assert!(!pattern.matches("/blog/hello/world"));
    }

    #[test]
    fn test_wildcard_match() {
        let pattern = PathPattern::parse("/blog/:path*").unwrap();
        assert!(pattern.matches("/blog"));
        assert!(pattern.matches("/blog/"));
        assert!(pattern.matches("/blog/post"));
        assert!(pattern.matches("/blog/post/123"));
        assert!(pattern.matches("/blog/a/b/c/d"));
        assert!(!pattern.matches("/blogs"));
    }

    #[test]
    fn test_root_match() {
        let pattern = PathPattern::parse("/").unwrap();
        assert!(pattern.matches("/"));
        assert!(!pattern.matches("/blog"));
    }

    #[test]
    fn test_complex_pattern() {
        let pattern = PathPattern::parse("/api/:version/users/:id").unwrap();
        assert!(pattern.matches("/api/v1/users/123"));
        assert!(pattern.matches("/api/v2/users/456"));
        assert!(!pattern.matches("/api/v1/users"));
        assert!(!pattern.matches("/api/v1/users/123/posts"));
    }

    #[test]
    fn test_wildcard_after_segments() {
        let pattern = PathPattern::parse("/docs/:path*").unwrap();
        assert!(pattern.matches("/docs"));
        assert!(pattern.matches("/docs/getting-started"));
        assert!(pattern.matches("/docs/api/reference"));
    }

    #[test]
    fn test_pattern_parse_errors() {
        let err = PathPattern::parse("").unwrap_err();
        assert!(err.contains("cannot be empty"));

        let err = PathPattern::parse("/api/:").unwrap_err();
        assert!(err.contains("Parameter name cannot be empty"));
    }

    #[test]
    fn test_multiple_exact_segments() {
        let pattern = PathPattern::parse("/api/v1/users").unwrap();
        assert!(pattern.matches("/api/v1/users"));
        assert!(!pattern.matches("/api/v1/posts"));
        assert!(!pattern.matches("/api/v1"));
        assert!(!pattern.matches("/api/v1/users/123"));
    }

    #[test]
    fn test_exact_match_precedence_over_wildcard() {
        let pattern_specific = PathPattern::parse("/blog").unwrap();
        let pattern_wildcard = PathPattern::parse("/:path*").unwrap();

        assert!(pattern_specific.matches("/blog"));
        assert!(pattern_wildcard.matches("/blog"));
        assert!(!pattern_specific.matches("/other"));
        assert!(pattern_wildcard.matches("/other"));
    }

    #[test]
    fn test_param_match_precedence_over_wildcard() {
        let pattern_param = PathPattern::parse("/user/:id").unwrap();
        let pattern_wildcard = PathPattern::parse("/:path*").unwrap();

        assert!(pattern_param.matches("/user/123"));
        assert!(pattern_wildcard.matches("/user/123"));
        assert!(!pattern_param.matches("/post/abc"));
        assert!(pattern_wildcard.matches("/post/abc"));
    }
}
