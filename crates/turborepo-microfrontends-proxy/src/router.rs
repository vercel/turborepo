use std::collections::HashMap;

use turborepo_microfrontends::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    pub app_name: String,
    pub port: u16,
}

#[derive(Clone)]
pub struct Router {
    routes: Vec<Route>,
    default_app: RouteMatch,
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
                default_app = Some(RouteMatch {
                    app_name: app_name.to_string(),
                    port,
                });
            }
        }

        let default_app = default_app.ok_or_else(|| {
            "No default application found (application without routing configuration)".to_string()
        })?;

        Ok(Self {
            routes,
            default_app,
        })
    }

    pub fn match_route(&self, path: &str) -> RouteMatch {
        for route in &self.routes {
            for pattern in &route.patterns {
                if pattern.matches(path) {
                    return RouteMatch {
                        app_name: route.app_name.clone(),
                        port: route.port,
                    };
                }
            }
        }

        self.default_app.clone()
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
