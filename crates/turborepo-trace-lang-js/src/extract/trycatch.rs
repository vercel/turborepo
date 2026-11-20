//! Which `try` blocks swallow their errors.
//!
//! A module load inside `try` is marked [`RefFlags::OPTIONAL`] by the extractor
//! (the lexer's `IN_TRY` flag). The failure is only tolerated when the `catch`
//! clause swallows it, so after extraction the flag is cleared again on module
//! loads inside a `try` whose handler may rethrow: no `catch` clause (`try {}
//! finally {}`), or a `catch` body containing `throw` anywhere (conservative: a
//! `throw` in a nested function also counts).
//!
//! [`RefFlags::OPTIONAL`]: turborepo_trace_core::RefFlags::OPTIONAL

use turborepo_trace_core::{RawRef, RefFlags, RefKind};

use crate::{
    Dialect,
    lexer::{Kw, Lexer, TokKind, Token},
};

/// Every `try` block: its byte range (`{` to `}`) and whether its errors may
/// propagate.
pub(crate) fn try_blocks(src: &[u8], dialect: Dialect) -> Vec<(u32, u32, bool)> {
    let mut lx = Lexer::new(src, dialect);
    let mut toks: Vec<Token> = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof {
            break;
        }
        toks.push(t);
    }
    // Matching `}` for each `{`.
    let mut close = vec![usize::MAX; toks.len()];
    let mut stack = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokKind::LBrace => stack.push(i),
            TokKind::RBrace => {
                if let Some(o) = stack.pop() {
                    close[o] = i;
                }
            }
            _ => {}
        }
    }
    // `throws[i]`: `throw` keywords among the first `i` tokens (linear handler
    // checks).
    let mut throws = Vec::with_capacity(toks.len() + 1);
    throws.push(0u32);
    for t in &toks {
        let n = *throws.last().unwrap_or(&0);
        throws.push(n + u32::from(t.kind == TokKind::Ident && t.kw == Kw::Throw));
    }
    let is_word = |i: usize, w: &[u8]| {
        toks.get(i)
            .is_some_and(|t| t.kind == TokKind::Ident && t.text(src) == w)
    };
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if toks[i].kind != TokKind::Ident || toks[i].kw != Kw::Try {
            continue;
        }
        let open = i + 1;
        if toks.get(open).map(|t| t.kind) != Some(TokKind::LBrace) {
            continue;
        }
        let end = close[open];
        if end == usize::MAX {
            // Unterminated: keep it optional only if we can tell; we can't.
            out.push((toks[open].start, u32::MAX, true));
            continue;
        }
        let range = (toks[open].start, toks[end].end);
        let mut k = end + 1;
        if !is_word(k, b"catch") {
            out.push((range.0, range.1, true));
            continue;
        }
        k += 1;
        // `catch (e)` / `catch ({ message })`.
        if toks.get(k).map(|t| t.kind) == Some(TokKind::LParen) {
            let mut depth = 0i32;
            while k < toks.len() {
                match toks[k].kind {
                    TokKind::LParen => depth += 1,
                    TokKind::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            k += 1;
        }
        if toks.get(k).map(|t| t.kind) != Some(TokKind::LBrace) || close[k] == usize::MAX {
            out.push((range.0, range.1, true));
            continue;
        }
        let rethrows = throws[close[k]] > throws[k + 1];
        out.push((range.0, range.1, rethrows));
    }
    out
}

/// Clears [`RefFlags::OPTIONAL`] on module loads whose innermost enclosing
/// `try` may rethrow. Only references without an uncertainty of their own are
/// touched (special cases set `OPTIONAL` for other reasons and carry one).
pub(crate) fn clear_unswallowed(src: &[u8], dialect: Dialect, refs: &mut [RawRef<'_>]) {
    let is_load = |r: &RawRef<'_>| {
        matches!(
            r.kind,
            RefKind::Require | RefKind::Import | RefKind::DynamicImport | RefKind::ReExport
        ) && r.flags.contains(RefFlags::OPTIONAL)
            && r.uncertainty.is_none()
    };
    if !refs.iter().any(is_load) {
        return;
    }
    // Blocks are in source order and nest, so one sweep with a stack of the open
    // blocks finds each reference's innermost block.
    let blocks = try_blocks(src, dialect);
    let mut order: Vec<usize> = (0..refs.len()).filter(|&i| is_load(&refs[i])).collect();
    order.sort_by_key(|&i| refs[i].span.start);
    let mut open: Vec<(u32, u32, bool)> = Vec::new();
    let mut next = 0;
    for i in order {
        let at = refs[i].span.start;
        while next < blocks.len() && blocks[next].0 <= at {
            while open.last().is_some_and(|b| b.1 <= blocks[next].0) {
                open.pop();
            }
            open.push(blocks[next]);
            next += 1;
        }
        while open.last().is_some_and(|b| b.1 <= at) {
            open.pop();
        }
        if open.last().is_some_and(|b| b.2) {
            refs[i].flags.remove(RefFlags::OPTIONAL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(src: &str) -> Vec<&str> {
        try_blocks(src.as_bytes(), Dialect::JS)
            .into_iter()
            .filter(|b| b.2)
            .map(|(a, b, _)| &src[a as usize..(b as usize).min(src.len())])
            .collect()
    }

    #[test]
    fn swallowing_and_rethrowing_handlers() {
        assert!(blocks("try { a } catch {}").is_empty());
        assert!(blocks("try { a } catch (e) { log(e) } finally { b }").is_empty());
        assert_eq!(blocks("try { a } finally { b }"), ["{ a }"]);
        assert_eq!(blocks("try { a } catch (e) { throw e }"), ["{ a }"]);
        assert_eq!(
            blocks("try { a } catch ({ code }) { if (code) { throw new Error() } }"),
            ["{ a }"]
        );
        // Nested: the inner one swallows, the outer one rethrows.
        assert_eq!(
            blocks("try { try { a } catch {} } catch (e) { throw e }"),
            ["{ try { a } catch {} }"]
        );
    }
}
