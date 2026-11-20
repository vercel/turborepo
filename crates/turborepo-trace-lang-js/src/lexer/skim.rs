//! The block skim behind [`Lexer::next_matching_top`].
//!
//! # Idea
//!
//! Most bytes of a JS file cannot change the lexer's state: identifiers,
//! whitespace and most operators. Only a few bytes can ("events"): quotes and
//! backtick (literals), `/` (comment, regex or division), `( ) [ ] { }` (the
//! context stack and template resumption), `?` and `:` (ternaries, `case`
//! labels), non-ASCII bytes (Unicode whitespace or identifiers), and, by
//! dialect, `!` (TypeScript non-null assertion) and `<` (JSX). The skim
//! classifies 64 bytes at a time into an event mask and an identifier mask
//! ([`turborepo_trace_core::lexkit::blocks`], SIMD) and jumps from event to
//! event. Identifiers between events matter only if they may be one of
//! the watched words (trigger keywords, dynamic trigger words, and `case` /
//! `default`, which open a label): SIMD sets of their first and second bytes
//! (and of the bytes of one-letter words) select candidate word starts, which a
//! first-byte/length filter then checks. Everything else is never tokenized.
//!
//! # Previous significant token, lazily
//!
//! Some events depend on the previous significant token: `/` and `<` (regex or
//! JSX allowed), `{` (block or object), `(` (after `if` / `while` / `for` /
//! `with` / `for await`), `!` (TS postfix), triggers (the keyword is void after
//! `.`, `TYPE_POS`, `prev_kw`) and the end of input. For those, [`Lexer::lst`]
//! reconstructs that token from the bytes just before the event: the last
//! non-whitespace byte is the end of either an identifier-like word (whose
//! keyword needs the token before it: `a.if` vs `if`) or an operator run, which
//! is re-lexed forward with the same rules as the full lexer ([`op_tok`]).
//! Events that do not depend on it (`)`, `]`, `[`, `}`, `?`, `:`, literals)
//! just commit their own token.
//!
//! The state at `q` (the end of the last committed token) is always exact.
//! Whenever the reconstruction would be ambiguous (a number swallowing a `.` or
//! an exponent sign, `await` before `(`, a word glued to a non-ASCII byte), the
//! skim rewinds to `q` and re-lexes that run with the full lexer
//! ([`Lexer::skim_exact_until`]). Runs are disjoint, so the total work stays
//! linear.

use std::sync::LazyLock;

use turborepo_trace_core::lexkit::blocks::{BLOCK, ByteSet, masks_n};

use super::{
    CtxKind, Jsx, Kw, Lexer, TokKind, Token,
    tables::{
        IDENT_CLASS, IDENT_START, SK_COND_LAST, SK_DIGIT, SK_ID, SK_NL, SK_WS, SKIM, keyword,
        keyword_text,
    },
};

/// ASCII identifier bytes.
// Invariant: a fixed byte set that fits the tables (unit-tested).
#[allow(clippy::expect_used)]
static ID_SET: LazyLock<ByteSet> = LazyLock::new(|| {
    ByteSet::new(|b| IDENT_CLASS[b as usize] == 1).expect("identifier set fits nibble tables")
});

/// Event bytes per dialect, indexed by `ts as usize * 2 + jsx as usize`.
// Invariant: fixed byte sets that fit the tables (unit-tested).
#[allow(clippy::expect_used)]
static EV_SETS: LazyLock<[ByteSet; 4]> = LazyLock::new(|| {
    [(false, false), (false, true), (true, false), (true, true)]
        .map(|(ts, jsx)| ByteSet::new(|b| is_event(b, ts, jsx)).expect("event set fits"))
});

fn is_event(b: u8, ts: bool, jsx: bool) -> bool {
    matches!(
        b,
        b'\'' | b'"' | b'`' | b'/' | b'{' | b'}' | b'(' | b')' | b'[' | b']' | b'?' | b':'
    ) || b >= 0x80
        || (ts && b == b'!')
        || (jsx && b == b'<')
}

#[inline(always)]
fn class(b: u8) -> u8 {
    SKIM[b as usize]
}

#[inline(always)]
fn is_id(b: u8) -> bool {
    class(b) & SK_ID != 0
}

#[inline(always)]
fn is_ws(b: u8) -> bool {
    class(b) & SK_WS != 0
}

/// Whether the identifier-byte run ending at `i` (inclusive, not extending
/// below `q`) may be the end of a number token. Numbers consume letters, one
/// `.` and exponent signs (`1e+x.y` is a single number), so runs joined by `.`
/// or `e+` / `e-` are followed back to a digit. Conservative: may say yes for
/// non-numbers, never no for numbers.
#[inline]
fn maybe_number_ending(src: &[u8], q: usize, i: usize) -> bool {
    let mut i = i;
    for _ in 0..8 {
        let mut k = i;
        while k > q && is_id(src[k - 1]) {
            k -= 1;
        }
        if class(src[k]) & SK_DIGIT != 0 {
            return true;
        }
        if k < q + 2 {
            return false;
        }
        let (b, p) = (src[k - 1], src[k - 2]);
        let join =
            is_id(p) && (b == b'.' || (matches!(b, b'+' | b'-') && matches!(p, b'e' | b'E')));
        if !join {
            return false;
        }
        i = k - 2;
    }
    true
}

/// Whether the `.`, `+` or `-` at `j` may be swallowed by a number token ending
/// just before it (`1.`, `1e+`).
#[inline(always)]
fn joins_number(src: &[u8], q: usize, j: usize) -> bool {
    if j <= q || !is_id(src[j - 1]) {
        return false;
    }
    let ok = match src[j] {
        b'.' => true,
        b'+' | b'-' => matches!(src[j - 1], b'e' | b'E'),
        _ => false,
    };
    ok && maybe_number_ending(src, q, j - 1)
}

/// Length and kind of the operator token at `i`, for bytes that start neither
/// an identifier, number, literal, bracket, comment nor regex: the operator
/// cases of [`Lexer::normal`] (`.` is never followed by a digit here, `#` never
/// by an identifier start, `<` is not JSX, and the TypeScript postfix-`!`
/// decision is the caller's).
#[inline(always)]
pub(super) fn op_tok(src: &[u8], i: usize) -> (usize, TokKind) {
    let b = src[i];
    let n1 = src.get(i + 1).copied();
    let n2 = src.get(i + 2).copied();
    match b {
        b';' => (1, TokKind::Semi),
        b',' => (1, TokKind::Comma),
        b':' => (1, TokKind::Colon),
        b'.' => match (n1, n2) {
            (Some(b'.'), Some(b'.')) => (3, TokKind::Ellipsis),
            _ => (1, TokKind::Dot),
        },
        b'=' => match (n1, n2) {
            (Some(b'>'), _) => (2, TokKind::Arrow),
            (Some(b'='), Some(b'=')) => (3, TokKind::Op),
            (Some(b'='), _) => (2, TokKind::Op),
            _ => (1, TokKind::Assign),
        },
        b'!' => match (n1, n2) {
            (Some(b'='), Some(b'=')) => (3, TokKind::Op),
            (Some(b'='), _) => (2, TokKind::Op),
            _ => (1, TokKind::Bang),
        },
        b'<' => match (n1, n2) {
            (Some(b'<'), Some(b'=')) => (3, TokKind::Op),
            (Some(b'<' | b'='), _) => (2, TokKind::Op),
            _ => (1, TokKind::Lt),
        },
        b'>' => (1, TokKind::Gt),
        b'+' | b'-' => match n1 {
            Some(c) if c == b => (2, TokKind::IncDec),
            Some(b'=') => (2, TokKind::Op),
            _ => (1, TokKind::Op),
        },
        b'*' => match (n1, n2) {
            (Some(b'*'), Some(b'=')) => (3, TokKind::Op),
            (Some(b'*' | b'='), _) => (2, TokKind::Op),
            _ => (1, TokKind::Star),
        },
        b'&' | b'|' => match (n1, n2) {
            (Some(c), Some(b'=')) if c == b => (3, TokKind::Op),
            (Some(c), _) if c == b || c == b'=' => (2, TokKind::Op),
            _ => (1, TokKind::Op),
        },
        b'%' | b'^' => match n1 {
            Some(b'=') => (2, TokKind::Op),
            _ => (1, TokKind::Op),
        },
        b'@' => (1, TokKind::At),
        b'#' => (1, TokKind::Hash),
        _ => (1, TokKind::Op),
    }
}

/// The previous significant token at some position, reconstructed by
/// [`Lexer::lst`].
#[derive(Copy, Clone, Debug)]
struct Lst {
    /// The token's kind and keyword, or `None` if no token lies between the
    /// last committed one and the position.
    tok: Option<(TokKind, Kw)>,
    /// Whether a line terminator lies between that token and the position.
    nl: bool,
}

enum Step {
    /// Keep skimming from `self.pos` (state committed through it).
    Next,
    /// As [`Step::Next`], after tokens from the full lexer (which may end in an
    /// identifier byte).
    NextLexed,
    /// Return this token.
    Hit(Token),
    /// The run is ambiguous: re-lex it with the full lexer.
    Exact,
    /// JSX mode was entered.
    Mode,
}

impl Lexer<'_> {
    /// Builds `skim_filter` and `skim_first` for `(mask, top_mask)`
    /// (normalized).
    fn build_skim_filter(&mut self, mask: u64, top_mask: u64) {
        let mut f = [0u16; 256];
        let mut second = [false; 256];
        let words = (mask | top_mask | (1 << Kw::Case as u8) | (1 << Kw::Default as u8)) & !1;
        for k in 1..64u8 {
            if words & (1u64 << k) == 0 {
                continue;
            }
            if let Some(kw) = kw_from_u8(k) {
                let t = keyword_text(kw);
                f[t[0] as usize] |= 1 << t.len().min(15);
                second[t[1] as usize] = true;
            }
        }
        let mut short = [false; 256];
        if mask & 1 != 0
            && let Some(d) = &self.dyn_filter
        {
            for (fe, de) in f.iter_mut().zip(d.iter()) {
                *fe |= de;
            }
            for (b, s) in second.iter_mut().enumerate() {
                *s |= self.dyn_second[b / 64] & (1 << (b % 64)) != 0;
            }
            for (s, de) in short.iter_mut().zip(d.iter()) {
                *s = de & (1 << 1) != 0;
            }
        }
        // Words are found through ASCII identifier bytes only (a non-ASCII byte is an
        // event that re-lexes exactly), so only the ASCII parts of the sets
        // matter; they always fit the nibble tables (at most 8 rows).
        self.skim_first = ByteSet::new(|b| b < 0x80 && f[b as usize] != 0).unwrap_or(*ID_SET);
        self.skim_second = ByteSet::new(|b| b < 0x80 && second[b as usize]).unwrap_or(*ID_SET);
        self.skim_short_first = ByteSet::new(|b| b < 0x80 && short[b as usize]).unwrap_or(*ID_SET);
        self.skim_filter = f;
        self.skim_key = Some((mask, top_mask));
    }

    /// Event mask, identifier mask and candidate-word mask of the block at
    /// `base`. Candidates are word starts whose first byte is in
    /// `skim_first` and whose second byte is in `skim_second`, or one-byte
    /// words whose byte is in `skim_short_first`. `carry` is the identifier
    /// bit of the byte before the block (0 at a token boundary, where a word
    /// may start after any byte).
    #[inline(always)]
    // Invariant: the slice is exactly BLOCK bytes (`get(base..base + BLOCK)`).
    #[allow(clippy::expect_used)]
    fn skim_block(&self, base: usize, carry: u64, ev_set: &ByteSet) -> (u64, u64, u64) {
        let src = self.src;
        let sets = [
            ev_set,
            &*ID_SET,
            &self.skim_first,
            &self.skim_second,
            &self.skim_short_first,
        ];
        let [ev, id, first, second, short] = match src.get(base..base + BLOCK) {
            Some(b) => masks_n(b.try_into().expect("block length"), sets),
            None => {
                // Past the end: padding (no events, no identifiers), and the end itself is
                // an event.
                let mut buf = [b' '; BLOCK];
                let n = src.len() - base;
                buf[..n].copy_from_slice(&src[base..]);
                let mut m = masks_n(&buf, sets);
                m[0] |= 1u64 << n;
                m
            }
        };
        let starts = id & !((id << 1) | carry);
        // The byte after the last one is in the next block: let it through.
        let next = id >> 1;
        let long = first & ((second >> 1) | (1 << 63));
        let one = short & !next;
        (ev, id, starts & (long | one))
    }

    /// The block skim. `mask` / `top_mask` are normalized (see
    /// `next_matching_top`).
    pub(super) fn skim(&mut self, mask: u64, top_mask: u64) -> Token {
        if self.skim_key != Some((mask, top_mask)) {
            self.build_skim_filter(mask, top_mask);
        }
        let ev_set =
            EV_SETS[usize::from(self.dialect.ts) * 2 + usize::from(self.dialect.jsx != Jsx::Off)];
        let src = self.src;
        'outer: loop {
            while self.jsx_top {
                let t = self.next_token_inline();
                if self.is_match(&t, mask, top_mask) {
                    return t;
                }
            }
            // The state is exact at `q`. `ev` and `cand` hold the events and candidate
            // words of the block at `base` that are not yet processed.
            let mut q = self.pos;
            let mut base = q;
            let (mut ev, mut id, mut cand) = self.skim_block(base, 0, &ev_set);
            loop {
                // Candidate words before the next event (all of them if there is none).
                let before = ev.wrapping_sub(1) & !ev;
                while cand & before != 0 {
                    let off = cand.trailing_zeros() as usize;
                    cand &= cand - 1;
                    let w = base + off;
                    let ones = (!(id >> off)).trailing_zeros() as usize;
                    let mut e = w + ones;
                    if off + ones >= BLOCK {
                        while e < src.len() && is_id(src[e]) {
                            e += 1;
                        }
                    }
                    if self.skim_filter[src[w] as usize] & (1 << (e - w).min(15)) == 0 {
                        continue;
                    }
                    match self.skim_word(q, w, e, mask, top_mask) {
                        Step::Next | Step::NextLexed | Step::Mode => {}
                        Step::Hit(t) => return t,
                        Step::Exact => {
                            if let Some(t) = self.skim_exact_until(q, w, mask, top_mask) {
                                return t;
                            }
                            continue 'outer;
                        }
                    }
                }
                if ev == 0 {
                    let carry = id >> 63;
                    base += BLOCK;
                    (ev, id, cand) = self.skim_block(base, carry, &ev_set);
                    continue;
                }
                let p = base + ev.trailing_zeros() as usize;
                let glued = match self.skim_event(q, p, mask, top_mask) {
                    Step::Next => false,
                    Step::NextLexed => true,
                    Step::Hit(t) => return t,
                    Step::Exact => {
                        if let Some(t) = self.skim_exact_until(q, p, mask, top_mask) {
                            return t;
                        }
                        continue 'outer;
                    }
                    Step::Mode => continue 'outer,
                };
                q = self.pos;
                let rel = q - base;
                // After a token from the full lexer, a word may start right after an
                // identifier byte (`1$a`: the number ends at `$`), which the block's word
                // starts miss.
                if rel >= BLOCK
                    || (glued && is_id(src[q - 1]) && src.get(q).is_some_and(|&b| is_id(b)))
                {
                    base = q;
                    (ev, id, cand) = self.skim_block(base, 0, &ev_set);
                } else if rel == p - base + 1 {
                    // A one-byte event token.
                    ev &= ev - 1;
                } else {
                    let keep = u64::MAX << rel;
                    ev &= keep;
                    cand &= keep;
                }
            }
        }
    }

    /// Re-lexes from `q` (where the state is exact) with the full lexer until
    /// the token at `stop` has been consumed. Returns a matching token if
    /// one is reached.
    #[cold]
    #[inline(never)]
    fn skim_exact_until(
        &mut self,
        q: usize,
        stop: usize,
        mask: u64,
        top_mask: u64,
    ) -> Option<Token> {
        self.pos = q;
        while self.pos <= stop {
            let t = if self.jsx_top {
                Some(self.next_token_inline())
            } else {
                self.normal()
            };
            if let Some(t) = t
                && self.is_match(&t, mask, top_mask)
            {
                return Some(t);
            }
        }
        None
    }

    /// A filtered identifier-byte word at `w..e` (in the run after `q`).
    #[inline(always)]
    fn skim_word(&mut self, q: usize, w: usize, e: usize, mask: u64, top_mask: u64) -> Step {
        let src = self.src;
        if src.get(e).is_some_and(|&c| c >= 0x80) {
            // The identifier continues with non-ASCII bytes (or ends at Unicode
            // whitespace).
            return Step::Exact;
        }
        if w > q {
            let b = src[w - 1];
            if b == b'#' {
                // `#word`: a private name.
                return Step::Next;
            }
            if joins_number(src, q, w - 1) {
                // `1.x`, `1e+x`: maybe one number token.
                return Step::Exact;
            }
        }
        let kw0 = keyword(&src[w..e]);
        // A keyword after `.` is a plain identifier, so it may still be a dynamic
        // trigger.
        let maybe = self.dyn_hit(w, e)
            || (kw0 != Kw::None
                && ((mask | top_mask) & (1u64 << kw0 as u8) != 0
                    || matches!(kw0, Kw::Case | Kw::Default)));
        if !maybe {
            return Step::Next;
        }
        let Some(l) = self.lst(q, w) else {
            return Step::Exact;
        };
        let last = l.tok.map_or(self.last, |t| t.0);
        let kw = if matches!(last, TokKind::Dot | TokKind::OptChain) {
            Kw::None
        } else {
            kw0
        };
        let bit = 1u64 << kw as u8;
        let hit = (mask & bit != 0 && (kw != Kw::None || self.dyn_hit(w, e)))
            || (top_mask & bit != 0 && self.stack.is_empty());
        if !hit {
            if matches!(kw, Kw::Case | Kw::Default) {
                self.case_pending = true;
            }
            return Step::Next;
        }
        self.apply_lst(l);
        self.pos = e;
        let flags = if kw != Kw::None {
            self.keyword_flags(kw)
        } else {
            0
        };
        let depth = self.stack.len();
        Step::Hit(self.tok(TokKind::Ident, w, depth, kw, flags))
    }

    /// The event byte at `p` (or the end of input), after the run `q..p`.
    #[inline(always)]
    fn skim_event(&mut self, q: usize, p: usize, mask: u64, top_mask: u64) -> Step {
        let src = self.src;
        let Some(&b) = src.get(p) else {
            let Some(l) = self.lst(q, p) else {
                return Step::Exact;
            };
            self.apply_lst(l);
            self.pos = src.len();
            return Step::Hit(self.eof());
        };
        match b {
            b'(' => {
                // Only `if`, `while`, `for`, `with` or `await` before it can make this a
                // condition paren: check their last two bytes first.
                let mut j = p;
                while j > q && is_ws(src[j - 1]) {
                    j -= 1;
                }
                let kind = if j == q {
                    self.paren_kind()
                } else if j >= q + 2
                    && class(src[j - 1]) & SK_COND_LAST != 0
                    && matches!(&src[j - 2..j], b"if" | b"le" | b"or" | b"th" | b"it")
                {
                    let Some(l) = self.lst(q, p) else {
                        return Step::Exact;
                    };
                    self.apply_lst(l);
                    self.paren_kind()
                } else {
                    // After some other token: a plain paren.
                    CtxKind::Paren
                };
                self.pos = p + 1;
                self.push(kind);
                self.commit(TokKind::LParen, Kw::None);
            }
            b')' => {
                self.pos = p + 1;
                self.close_paren(p);
                self.commit(TokKind::RParen, Kw::None);
            }
            b'[' => {
                self.pos = p + 1;
                self.push(CtxKind::Bracket);
                self.commit(TokKind::LBracket, Kw::None);
            }
            b']' => {
                self.pos = p + 1;
                self.close(p, &[CtxKind::Bracket], CtxKind::Paren);
                self.commit(TokKind::RBracket, Kw::None);
            }
            b'{' => {
                let Some(l) = self.lst(q, p) else {
                    return Step::Exact;
                };
                self.apply_lst(l);
                self.pos = p + 1;
                self.open_brace();
                self.commit(TokKind::LBrace, Kw::None);
            }
            b'}' => {
                let k = self.top();
                if matches!(
                    k,
                    Some(CtxKind::Block | CtxKind::TryBlock | CtxKind::Object)
                ) {
                    self.pos = p + 1;
                    self.pop();
                    self.last_brace_block = k != Some(CtxKind::Object);
                    self.commit(TokKind::RBrace, Kw::None);
                } else {
                    self.close_brace_slow(p);
                    if self.jsx_top {
                        return Step::Mode;
                    }
                }
            }
            b'?' => {
                let (len, kind) = match (src.get(p + 1), src.get(p + 2)) {
                    (Some(b'.'), c) if !matches!(c, Some(b'0'..=b'9')) => (2, TokKind::OptChain),
                    (Some(b'?'), Some(b'=')) => (3, TokKind::Op),
                    (Some(b'?'), _) => (2, TokKind::Op),
                    _ => {
                        self.ternary += 1;
                        (1, TokKind::Question)
                    }
                };
                self.pos = p + len;
                self.commit(kind, Kw::None);
            }
            b':' => {
                self.pos = p + 1;
                self.colon();
                self.commit(TokKind::Colon, Kw::None);
            }
            b'"' | b'\'' => {
                self.string(p, b);
            }
            b'`' => {
                self.pos = p + 1;
                let depth = self.stack.len();
                self.template(p, depth, TokKind::Template, TokKind::TemplateHead);
            }
            b'/' | b'<' | b'!' => {
                let Some(l) = self.lst(q, p) else {
                    return Step::Exact;
                };
                self.apply_lst(l);
                self.pos = p;
                if let Some(t) = self.normal()
                    && self.is_match(&t, mask, top_mask)
                {
                    return Step::Hit(t);
                }
                if self.jsx_top {
                    return Step::Mode;
                }
                return Step::NextLexed;
            }
            // Non-ASCII: Unicode whitespace or identifier bytes.
            _ => return Step::Exact,
        }
        Step::Next
    }

    /// Commits a reconstructed previous token (see [`Lexer::lst`]).
    #[inline(always)]
    fn apply_lst(&mut self, l: Lst) {
        if let Some((kind, kw)) = l.tok {
            // `prev_kw` becomes stale here; it is only read when `last_kw` is `await`,
            // which `lst` never reconstructs.
            self.commit(kind, kw);
            if kind == TokKind::Bang {
                // Only JS reaches this (`!` is an event in TypeScript): never a postfix.
                self.last_bang_postfix = false;
            }
        }
        self.newline = l.nl;
    }

    /// The previous significant token at `x`, where `q..x` contains no events
    /// (and so no literals or comments). `None` if it cannot be determined
    /// cheaply and exactly.
    #[inline(always)]
    fn lst(&self, q: usize, x: usize) -> Option<Lst> {
        let src = self.src;
        let mut e = x;
        while e > q && is_ws(src[e - 1]) {
            e -= 1;
        }
        if e == q {
            return Some(Lst {
                tok: None,
                nl: self.newline || has_nl(&src[q..x]),
            });
        }
        let nl = has_nl(&src[e..x]);
        let c = src[e - 1];
        let tok = if is_id(c) {
            let mut s = e - 1;
            while s > q && is_id(src[s - 1]) {
                s -= 1;
            }
            if class(src[s]) & SK_DIGIT != 0 {
                (TokKind::Number, Kw::None)
            } else {
                self.word_tok(q, s, e)?
            }
        } else {
            (self.op_run_last(q, e)?, Kw::None)
        };
        if tok.1 == Kw::Await {
            return None;
        }
        Some(Lst { tok: Some(tok), nl })
    }

    /// Kind and keyword of the identifier-like word `s..e` (not starting with a
    /// digit) that is the last token of a run starting at `q`.
    #[inline(always)]
    fn word_tok(&self, q: usize, s: usize, e: usize) -> Option<(TokKind, Kw)> {
        let src = self.src;
        let kw0 = keyword(&src[s..e]);
        if kw0 == Kw::None {
            // A plain identifier, a private name (`#x`) or the tail of a number (`1.x`):
            // all values without a keyword, which the lexer treats alike.
            return Some((TokKind::Ident, Kw::None));
        }
        if s > q {
            let b = src[s - 1];
            if b == b'#' {
                return Some((TokKind::PrivateName, Kw::None));
            }
            if joins_number(src, q, s - 1) {
                // One number token, or a keyword after `.` / an operator.
                return None;
            }
        }
        // A keyword is a plain identifier after `.` / `?.`.
        let mut j = s;
        while j > q && is_ws(src[j - 1]) {
            j -= 1;
        }
        let after_dot = if j == q {
            matches!(self.last, TokKind::Dot | TokKind::OptChain)
        } else if src[j - 1] == b'.' {
            self.op_run_last(q, j)? == TokKind::Dot
        } else {
            false
        };
        Some((TokKind::Ident, if after_dot { Kw::None } else { kw0 }))
    }

    /// The kind of the last token of the operator run ending at `e` (`src[e -
    /// 1]` is an operator byte), re-lexed forward from the run's start.
    #[inline(always)]
    fn op_run_last(&self, q: usize, e: usize) -> Option<TokKind> {
        let src = self.src;
        let mut r = e - 1;
        while r > q && !is_id(src[r - 1]) && !is_ws(src[r - 1]) {
            r -= 1;
        }
        if joins_number(src, q, r) {
            return None;
        }
        let mut i = r;
        let mut kind = TokKind::Op;
        while i < e {
            if src[i] == b'#' && src.get(i + 1).is_some_and(|&c| IDENT_START[c as usize]) {
                return None;
            }
            let (n, k) = op_tok(src, i);
            i += n;
            kind = k;
        }
        (i == e).then_some(kind)
    }
}

#[inline(always)]
fn has_nl(s: &[u8]) -> bool {
    s.iter().any(|&c| class(c) & SK_NL != 0)
}

/// The keyword with discriminant `k`, if any.
fn kw_from_u8(k: u8) -> Option<Kw> {
    const ALL: [Kw; 37] = [
        Kw::None,
        Kw::Import,
        Kw::Export,
        Kw::Require,
        Kw::Module,
        Kw::From,
        Kw::As,
        Kw::Type,
        Kw::Defer,
        Kw::Source,
        Kw::Const,
        Kw::Let,
        Kw::Var,
        Kw::Function,
        Kw::Class,
        Kw::Return,
        Kw::Typeof,
        Kw::Case,
        Kw::Do,
        Kw::Else,
        Kw::In,
        Kw::Instanceof,
        Kw::New,
        Kw::Delete,
        Kw::Void,
        Kw::Throw,
        Kw::Yield,
        Kw::Await,
        Kw::Of,
        Kw::Extends,
        Kw::Default,
        Kw::If,
        Kw::While,
        Kw::For,
        Kw::With,
        Kw::Try,
        Kw::Eval,
    ];
    ALL.get(k as usize)
        .copied()
        .filter(|&kw| kw != Kw::None && kw as u8 == k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Dialect;

    #[test]
    fn kw_table_is_complete() {
        for k in 1..37u8 {
            let kw = kw_from_u8(k).expect("keyword");
            assert_eq!(kw as u8, k);
        }
        assert_eq!(kw_from_u8(37), None);
        assert_eq!(kw_from_u8(0), None);
    }

    #[test]
    fn event_sets_are_exact() {
        for (i, set) in EV_SETS.iter().enumerate() {
            let (ts, jsx) = (i >= 2, i % 2 == 1);
            for b in 0..=255u8 {
                assert_eq!(
                    set.contains(b),
                    is_event(b, ts, jsx),
                    "{b:#x} ts={ts} jsx={jsx}"
                );
            }
        }
        for b in 0..=255u8 {
            assert_eq!(ID_SET.contains(b), is_id(b));
        }
    }

    /// `op_tok` agrees with the full lexer on every operator sequence of up to
    /// three bytes (followed by a space or an identifier).
    #[test]
    fn op_tok_matches_full_lexer() {
        let alphabet: &[u8] = b";,.=!<>+-*&|%^@#~\x01\x7f";
        for d in [Dialect::JS, Dialect::TS] {
            for &a in alphabet {
                for &b in alphabet.iter().chain(b" x") {
                    for &c in alphabet.iter().chain(b" x") {
                        // After a value, so `<` is not JSX (and `#!` is no hashbang).
                        let src = [b'x', b' ', a, b, c, b' ', b'y'];
                        if a == b'#' && IDENT_START[b as usize] {
                            continue;
                        }
                        let mut lx = Lexer::new(&src, d);
                        lx.next_token();
                        let t = lx.next_token();
                        let (n, k) = op_tok(&src, 2);
                        assert_eq!(
                            (t.end as usize - 2, t.kind),
                            (n, k),
                            "{:?}",
                            src.escape_ascii()
                        );
                    }
                }
            }
        }
    }
}
