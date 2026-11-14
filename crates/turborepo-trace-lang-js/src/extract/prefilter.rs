//! Keyword prefilter: a file that
//! contains none of the trigger words cannot contain a reference, so it is not
//! lexed at all.

use std::sync::LazyLock;

use memchr::memmem::Finder;

/// Trigger words. `port` covers `import` and `export` (and so `import.meta.url`
/// assets); `require` covers `require(`, `require.resolve` and
/// `module.require`; `reference` covers `/// <reference .../>`; `Worker` covers
/// a global `new Worker(...)`; `__dirname`, `__filename` and `cwd()` cover path
/// values emitted as assets without any import (see the deep pass in
/// [`super::deep`]). Runtime file APIs need an `import`/`require` of their
/// module. Files whose only dynamic construct is `eval` are skipped (no edge
/// would be produced anyway).
const TRIGGERS: [&str; 7] = [
    "port",
    "require",
    "reference",
    "Worker",
    "__dirname",
    "__filename",
    "cwd()",
];

static FINDERS: LazyLock<[Finder<'static>; 7]> =
    LazyLock::new(|| TRIGGERS.map(|t| Finder::new(t.as_bytes())));

/// Whether `src` may contain a reference. False positives are fine; false
/// negatives are bugs.
#[inline]
pub fn may_contain_refs(src: &[u8]) -> bool {
    FINDERS.iter().any(|f| f.find(src).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers() {
        for s in [
            &b"import x from 'y'"[..],
            b"export * from 'y'",
            b"require('x')",
            b"module.require('x')",
            b"/// <reference path=\"a\" />",
            b"x = import('y')",
            b"new Worker('./w.js')",
            b"return __dirname + '/x'",
            b"f(process.cwd())",
        ] {
            assert!(may_contain_refs(s), "{}", s.escape_ascii());
        }
        for s in [&b""[..], b"const x = 1;", b"function f() { return 2 }"] {
            assert!(!may_contain_refs(s));
        }
    }
}
