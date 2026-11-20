//! What a `stat` result is used for.
//!
//! `fs.stat` and `fs.lstat` return more than "is it there": the size and the
//! timestamps change when the file's contents change, so a `stat` is a content
//! read by default. Most calls only ask for the entry's kind, though
//! (`statSync(p).isDirectory()` while walking a tree). Those observe names only
//! ([`turborepo_trace_core::RefFlags::NAMES`]): no edit of a file can change
//! the answer, only adding or removing a path can.
//!
//! [`kind_only`] recognizes three shapes, on the source bytes around the call:
//!
//! - the kind is asked right away: `statSync(p).isDirectory()`, `(await
//!   stat(p)).isFile()`;
//! - the result is not used: `statSync(p);` as a statement (it throws or it
//!   doesn't);
//! - the result is bound with `const` and every occurrence of that name in the
//!   file asks for the kind: `const st = statSync(p); if (st.isDirectory()) …`.
//!
//! Anything else is not recognized, and the call stays a content read: a result
//! that is returned, passed on, destructured, compared, or read for `size`,
//! `mtimeMs`, `mode`; a `let` binding; a name that also occurs in a string or a
//! comment. The name rule looks at the whole file instead of the binding's
//! scope: whichever variable an occurrence refers to, an occurrence that only
//! asks for the kind cannot observe contents.

/// `fs.Stats` methods that depend only on the kind of the entry.
const KIND_METHODS: &[&[u8]] = &[
    b"isDirectory",
    b"isFile",
    b"isSymbolicLink",
    b"isFIFO",
    b"isSocket",
    b"isBlockDevice",
    b"isCharacterDevice",
];

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn skip_ws(src: &[u8], mut i: usize) -> usize {
    while i < src.len() && src[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn skip_ws_back(src: &[u8], mut i: usize) -> usize {
    while i > 0 && src[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

/// The identifier that ends at `end`, and where it starts.
fn ident_back(src: &[u8], end: usize) -> (&[u8], usize) {
    let mut s = end;
    while s > 0 && is_ident_byte(src[s - 1]) {
        s -= 1;
    }
    (&src[s..end], s)
}

/// Whether `.isDirectory(` (or another kind method, also after `?.`) starts at
/// `i`, whitespace aside.
fn kind_method_at(src: &[u8], i: usize) -> bool {
    let mut j = skip_ws(src, i);
    if src[j..].starts_with(b"?.") {
        j += 2;
    } else if src.get(j) == Some(&b'.') {
        j += 1;
    } else {
        return false;
    }
    j = skip_ws(src, j);
    let s = j;
    while j < src.len() && is_ident_byte(src[j]) {
        j += 1;
    }
    KIND_METHODS.contains(&&src[s..j]) && src.get(skip_ws(src, j)) == Some(&b'(')
}

/// Whether every occurrence of the variable `name` in `src`, other than
/// declarations, asks for the kind. Property names (`x.name`) are not
/// occurrences of the variable.
fn every_use_asks_kind(src: &[u8], name: &[u8]) -> bool {
    let n = name.len();
    let mut k = 0;
    while k + n <= src.len() {
        if &src[k..k + n] != name
            || (k > 0 && is_ident_byte(src[k - 1]))
            || src.get(k + n).is_some_and(|&b| is_ident_byte(b))
        {
            k += 1;
            continue;
        }
        let before = skip_ws_back(src, k);
        let property = before > 0 && src[before - 1] == b'.';
        let declared = matches!(ident_back(src, before).0, b"const" | b"let" | b"var");
        if !property && !declared && !kind_method_at(src, k + n) {
            return false;
        }
        k += n;
    }
    true
}

/// Whether the result of the `stat` call spanning `start..end` of `src` (from
/// the start of the callee to just past its closing parenthesis) is used only
/// for the kind of the entry, or not at all. See the module documentation.
pub fn kind_only(src: &[u8], start: usize, end: usize) -> bool {
    if start > end || end > src.len() {
        return false;
    }
    // `statSync(p).isDirectory()`
    if kind_method_at(src, end) {
        return true;
    }
    // An `await` in front of the call belongs to the expression.
    let mut expr = skip_ws_back(src, start);
    let awaited = ident_back(src, expr).0 == b"await";
    if awaited {
        expr = skip_ws_back(src, ident_back(src, expr).1);
    }
    let prev = expr.checked_sub(1).map(|i| src[i]);
    let after = skip_ws(src, end);
    let next = src.get(after).copied();
    // `(await stat(p)).isFile()`
    if awaited && prev == Some(b'(') && next == Some(b')') {
        return kind_method_at(src, after + 1);
    }
    // A statement of its own: the result is dropped.
    if matches!(prev, None | Some(b';' | b'{' | b'}')) && matches!(next, Some(b';' | b'}')) {
        return true;
    }
    // `const name = <call>` and nothing more in the initializer.
    let ends_statement =
        matches!(next, None | Some(b';' | b'}')) || src[end..after].contains(&b'\n');
    if prev != Some(b'=') || !ends_statement {
        return false;
    }
    let eq = expr - 1;
    if eq > 0
        && matches!(
            src[eq - 1],
            b'=' | b'!'
                | b'<'
                | b'>'
                | b'+'
                | b'-'
                | b'*'
                | b'/'
                | b'%'
                | b'&'
                | b'|'
                | b'^'
                | b'?'
        )
    {
        return false;
    }
    let (name, name_start) = ident_back(src, skip_ws_back(src, eq));
    if name.is_empty() || name[0].is_ascii_digit() {
        return false;
    }
    if ident_back(src, skip_ws_back(src, name_start)).0 != b"const" {
        return false;
    }
    every_use_asks_kind(src, name)
}

#[cfg(test)]
mod tests {
    use super::kind_only;

    /// `kind_only` for the first `statSync(`/`stat(`/`lstatSync(` call of
    /// `src`.
    fn check(src: &str) -> bool {
        let b = src.as_bytes();
        let call = [
            "fs.lstatSync(",
            "fs.statSync(",
            "fsp.stat(",
            "statSync(",
            "stat(",
        ]
        .iter()
        .find_map(|c| src.find(c))
        .expect("a stat call");
        let open = call + src[call..].find('(').unwrap();
        let mut depth = 0;
        let mut end = open;
        for (i, c) in src[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        kind_only(b, call, end)
    }

    #[test]
    fn kind_asked_right_away() {
        assert!(check("if (fs.statSync(p).isDirectory()) walk(p);"));
        assert!(check("return fs.lstatSync(p).isSymbolicLink();"));
        assert!(check("const d = fs.statSync(p)\n  ?.isFile();"));
        assert!(check("const d = (await fsp.stat(p)).isDirectory();"));
        assert!(!check("return fs.statSync(p).size;"));
        assert!(!check("return fs.statSync(p).mtimeMs > since;"));
        assert!(!check("const d = (await fsp.stat(p)).size;"));
        assert!(!check("const { size } = fs.statSync(p);"));
    }

    #[test]
    fn result_dropped() {
        assert!(check(
            "try { fs.statSync(p); return true; } catch { return false; }"
        ));
        assert!(check("try { await fsp.stat(p) } catch { return false }"));
        assert!(!check("return fs.statSync(p);"));
        assert!(!check("use(fs.statSync(p));"));
        assert!(!check("const all = [fs.statSync(p)];"));
    }

    #[test]
    fn bound_result_used_for_the_kind_only() {
        assert!(check(
            "const st = fs.statSync(p);\nif (st.isDirectory()) walk(p);\nelse if (st.isFile()) \
             out.push(p);"
        ));
        assert!(check(
            "const st = await fsp.stat(p)\nif (st?.isDirectory()) walk(p)"
        ));
        // Another variable of the same name is held to the same rule.
        assert!(check(
            "function a(p) { const st = fs.statSync(p); return st.isFile(); }\nfunction b(q) { \
             const st = fs.lstatSync(q); return st.isDirectory(); }"
        ));
        // A property called like the variable is not the variable.
        assert!(check(
            "const stat = fs.statSync(p);\nlog(o.stat);\nreturn stat.isFile();"
        ));
        // Size, timestamps, escapes, other bindings.
        assert!(!check(
            "const st = fs.statSync(p);\nreturn st.isFile() && st.size > 0;"
        ));
        assert!(!check("const st = fs.statSync(p);\nreturn st.mtimeMs;"));
        assert!(!check("const st = fs.statSync(p);\ncache.set(p, st);"));
        assert!(!check("const st = fs.statSync(p);\nreturn st;"));
        assert!(!check("const st = fs.statSync(p);\nconst { size } = st;"));
        assert!(!check("let st = fs.statSync(p);\nreturn st.isFile();"));
        assert!(!check("st = fs.statSync(p);\nreturn st.isFile();"));
        assert!(!check(
            "const st = fs.statSync(p) || fallback;\nreturn st.isFile();"
        ));
        assert!(!check(
            "const st: Stats = fs.statSync(p);\nreturn st.isFile();"
        ));
        assert!(!check(
            "const st = fs.statSync(p);\nreturn st.isFile; // not called"
        ));
        assert!(!check("const st = fs.statSync(p);\nreturn st['size'];"));
        // The name in a comment or a string counts as a use: not recognized.
        assert!(!check(
            "const st = fs.statSync(p); // st is fresh\nreturn st.isFile();"
        ));
    }

    #[test]
    fn comparisons_are_not_bindings() {
        assert!(!check("ok = a == fs.statSync(p);"));
        assert!(!check("const f = () => fs.statSync(p);"));
        assert!(!check("x += fs.statSync(p);"));
    }
}
