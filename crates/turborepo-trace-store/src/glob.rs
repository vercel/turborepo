//! Glob matching with exact prefilters, for matching one glob against many
//! paths.

/// A glob (`turborepo_trace_tasks::glob` semantics) split into its brace
/// alternatives, each with exact necessary conditions checked before the glob
/// runs: without `**`, a path has one component per segment and literal
/// segments match their component; with `**` (as whole segments), every literal
/// segment is some component. A literal tail of the last segment must end the
/// path.
pub(crate) struct FastGlob {
    alts: Vec<Alt>,
}

struct Alt {
    text: String,
    /// The alternative is `**`: it matches every path.
    all: bool,
    /// Leading wildcard-free segments: a match starts with these components.
    prefix: Vec<String>,
    /// `None`: no prefilter (escapes, classes, braces left, or `**` inside a
    /// segment).
    segs: Option<Vec<Option<String>>>,
    dstar: bool,
    tail: String,
}

impl FastGlob {
    pub(crate) fn new(pattern: &str) -> FastGlob {
        let alts = turborepo_trace_tasks::glob::expand_braces(pattern)
            .into_iter()
            .map(|text| {
                let pat = text.strip_prefix("./").unwrap_or(&text);
                let plain = !pat.contains(['{', '}', '\\', '[', ']'])
                    && pat.split('/').all(|seg| seg == "**" || !seg.contains("**"));
                let wild = |c: char| matches!(c, '*' | '?');
                let segs = plain.then(|| {
                    pat.split('/')
                        .map(|seg| (!seg.contains(wild)).then(|| seg.to_owned()))
                        .collect()
                });
                let last = pat.rsplit('/').next().unwrap_or(pat);
                let tail = if plain && !last.contains("**") {
                    last.rfind(wild).map_or(last, |i| &last[i + 1..]).to_owned()
                } else {
                    String::new()
                };
                let prefix = if plain {
                    pat.split('/')
                        .take_while(|seg| !seg.contains(wild) && !seg.is_empty())
                        .map(str::to_owned)
                        .collect()
                } else {
                    Vec::new()
                };
                Alt {
                    all: pat == "**",
                    prefix,
                    dstar: pat.contains("**"),
                    segs,
                    tail,
                    text,
                }
            })
            .collect();
        FastGlob { alts }
    }

    pub(crate) fn matches(&self, path: &str) -> bool {
        let p = path.strip_prefix("./").unwrap_or(path);
        self.alts.iter().any(|a| {
            if a.all {
                return true;
            }
            if !a.prefix.is_empty() {
                let mut comps = p.split('/');
                if !a.prefix.iter().all(|l| comps.next() == Some(l.as_str())) {
                    return false;
                }
            }
            if let Some(segs) = &a.segs {
                if !p.ends_with(a.tail.as_str()) {
                    return false;
                }
                if a.dstar {
                    let lits_ok = segs.iter().flatten().all(|l| p.split('/').any(|c| c == l));
                    if !lits_ok {
                        return false;
                    }
                } else {
                    let mut comps = p.split('/');
                    for s in segs {
                        match (comps.next(), s) {
                            (None, _) => return false,
                            (Some(c), Some(l)) if c != l => return false,
                            _ => {}
                        }
                    }
                    if comps.next().is_some() {
                        return false;
                    }
                }
            }
            turborepo_trace_tasks::glob::matches(&a.text, path)
        })
    }
}
