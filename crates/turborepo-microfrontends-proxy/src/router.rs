use std::collections::HashMap;

use turborepo_microfrontends::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    pub app_name: String,
    pub port: u16,
}

#[derive(Clone)]
pub struct Router {
    trie: TrieNode,
    apps: Vec<AppInfo>,
    default_app_idx: usize,
}

#[derive(Debug, Clone)]
struct AppInfo {
    app_name: String,
    port: u16,
}

#[derive(Clone, Default)]
struct TrieNode {
    exact_children: HashMap<String, TrieNode>,
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
    Exact(String),
    Param,
    Wildcard,
}

impl Router {
    pub fn new(config: &Config) -> Result<Self, String> {
        let mut routes = Vec::new();
        let mut default_app = None;
        let mut app_ports: HashMap<String, u16> = HashMap::new();

        for task in config.development_tasks() {
            let app_name = task.application_name;
            let port = config
                .port(app_name)
                .ok_or_else(|| format!("No port configured for application '{}'", app_name))?;

            app_ports.insert(app_name.to_string(), port);

            if let Some(routing) = config.routing(app_name) {
                let mut patterns = Vec::new();
                for path_group in routing {
                    for path in &path_group.paths {
                        patterns.push(PathPattern::parse(path)?);
                    }
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
            "No default application found (application without routing configuration)".to_string()
        })?;

        let mut apps = Vec::new();
        let mut trie = TrieNode::default();

        for route in routes {
            let app_idx = apps.len();
            apps.push(AppInfo {
                app_name: route.app_name,
                port: route.port,
            });

            for pattern in route.patterns {
                trie.insert(&pattern.segments, app_idx);
            }
        }

        let default_app_idx = apps.len();
        apps.push(AppInfo {
            app_name: default_app.0,
            port: default_app.1,
        });

        Ok(Self {
            trie,
            apps,
            default_app_idx,
        })
    }

    pub fn match_route(&self, path: &str) -> RouteMatch {
        let path = if path.starts_with('/') {
            &path[1..]
        } else {
            path
        };

        let app_idx = if path.is_empty() {
            self.trie.lookup(&[])
        } else {
            let mut segments = Vec::with_capacity(8);
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
                let child = self
                    .exact_children
                    .entry(name.clone())
                    .or_insert_with(TrieNode::default);
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

        if let Some(app_idx) = self.wildcard_match {
            return Some(app_idx);
        }

        if let Some(child) = self.exact_children.get(segments[0]) {
            if let Some(app_idx) = child.lookup(&segments[1..]) {
                return Some(app_idx);
            }
        }

        if let Some(child) = &self.param_child {
            if let Some(app_idx) = child.lookup(&segments[1..]) {
                return Some(app_idx);
            }
        }

        None
    }
}

impl PathPattern {
    fn parse(pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("Pattern cannot be empty".to_string());
        }

        let pattern = if pattern.starts_with('/') {
            &pattern[1..]
        } else {
            pattern
        };

        if pattern.is_empty() {
            return Ok(Self { segments: vec![] });
        }

        let mut segments = Vec::new();
        for segment in pattern.split('/') {
            if segment.is_empty() {
                continue;
            }

            if segment.starts_with(':') {
                let param_name = &segment[1..];
                if param_name.ends_with('*') {
                    segments.push(Segment::Wildcard);
                } else {
                    segments.push(Segment::Param);
                }
            } else {
                segments.push(Segment::Exact(segment.to_string()));
            }
        }

        Ok(Self { segments })
    }

    #[cfg(test)]
    fn matches(&self, path: &str) -> bool {
        let path = if path.starts_with('/') {
            &path[1..]
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
                    if path_segments[path_idx] != expected {
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
        assert!(PathPattern::parse("").is_err());
    }

    #[test]
    fn test_multiple_exact_segments() {
        let pattern = PathPattern::parse("/api/v1/users").unwrap();
        assert!(pattern.matches("/api/v1/users"));
        assert!(!pattern.matches("/api/v1/posts"));
        assert!(!pattern.matches("/api/v1"));
        assert!(!pattern.matches("/api/v1/users/123"));
    }
}
