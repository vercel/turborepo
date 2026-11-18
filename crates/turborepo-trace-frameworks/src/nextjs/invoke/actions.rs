//! Server actions: which files invoke which action.
//!
//! An action is an export of a module with the `'use server'` directive, or a
//! function whose body starts with it (an inline action). The import edge from
//! a client or server component to the action module is already in the graph;
//! what this adds is the relation "this file invokes this action" with the way
//! it does:
//!
//! | Use | `via` |
//! |---|---|
//! | `await save(data)` | `call` |
//! | `<form action={save}>`, `formAction={save}` | `form action` |
//! | `save.bind(null, id)` | `bind` |
//! | `useActionState(save, …)`, `useFormState`, `useAction(save)`, `startTransition(…)` | `hook:<name>` |
//! | passed on as a prop or stored | `reference` |
//! | imported and not used in the file (re-exported) | `import` |
//!
//! Every use counts: a reference to a server action in a component is a way to
//! invoke it. Exports are taken by name, so `export const save =
//! actionClient.schema(s).action(fn)` (safe-action style) is an action like any
//! other export of the module.

use std::{collections::HashSet, path::PathBuf, rc::Rc};

use turborepo_trace_lang_js::lexer::{Kw, TokKind};

use super::model::{InvokeKind, Site, SiteTarget};
use crate::nextjs::jseval::{Files, Imported, JsFile};

const ACTION_HOOKS: &[&str] = &[
    "useActionState",
    "useFormState",
    "useAction",
    "useOptimisticAction",
    "useStateAction",
    "useHookFormAction",
    "useServerAction",
    "startTransition",
];

fn is_action_module(f: &JsFile) -> bool {
    f.directive.as_deref() == Some("use server")
}

/// How the reference at token `t` uses the action.
fn how(f: &JsFile, t: usize) -> String {
    let next = f.kind(t + 1);
    if next == TokKind::LParen || next == TokKind::Lt && f.type_args_end(t + 1).is_some() {
        return "call".into();
    }
    if matches!(next, TokKind::Dot | TokKind::OptChain) && f.text(t + 2) == "bind" {
        return "bind".into();
    }
    // A JSX attribute: `action={save}` / `formAction={save}`.
    if let Some(tok) = f.toks.get(t) {
        let before = &f.src[..tok.start as usize];
        let trimmed = before
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .map_or(&before[..0], |i| &before[..=i]);
        if trimmed.ends_with(b"action={") || trimmed.ends_with(b"formAction={") {
            return "form action".into();
        }
    }
    // An argument of a hook.
    let mut o = f.open_of.get(t).copied().unwrap_or(u32::MAX);
    let mut hops = 0;
    while o != u32::MAX && hops < 4 {
        let o_ = o as usize;
        if f.kind(o_) == TokKind::LParen && o_ > 0 {
            let mut c = o_ - 1;
            if f.kind(c) == TokKind::Gt {
                // Type arguments: find the callee before them.
                let mut d = 0;
                while c > 0 {
                    match f.kind(c) {
                        TokKind::Gt => d += 1,
                        TokKind::Lt => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    c -= 1;
                }
                c = c.saturating_sub(1);
            }
            if f.kind(c) == TokKind::Ident && ACTION_HOOKS.contains(&f.text(c)) {
                return format!("hook:{}", f.text(c));
            }
        }
        o = f.open_of[o_];
        hops += 1;
    }
    "reference".into()
}

/// Whether token `t` (an identifier) is a specifier of an import or export
/// statement, a declaration, a property key or a member name rather than a use
/// of the binding.
fn not_a_use(f: &JsFile, t: usize) -> bool {
    let prev = if t > 0 { f.kind(t - 1) } else { TokKind::Eof };
    if matches!(prev, TokKind::Dot | TokKind::OptChain) {
        return true;
    }
    if f.kind(t + 1) == TokKind::Colon && matches!(prev, TokKind::LBrace | TokKind::Comma) {
        return true;
    }
    if t > 0
        && matches!(
            f.toks[t - 1].kw,
            Kw::Function | Kw::Const | Kw::Let | Kw::Var | Kw::Typeof | Kw::Import | Kw::As
        )
    {
        return true;
    }
    // Inside `import { … }` / `export { … }` / `import x, { … }`.
    let o = f.open_of.get(t).copied().unwrap_or(u32::MAX);
    if o != u32::MAX && f.kind(o as usize) == TokKind::LBrace && f.toks[o as usize].depth == 0 {
        let o = o as usize;
        let mut k = o;
        let mut steps = 0;
        while k > 0 && steps < 6 {
            k -= 1;
            steps += 1;
            match f.toks[k].kw {
                Kw::Import | Kw::Export => return true,
                _ => {}
            }
            if matches!(f.kind(k), TokKind::Semi | TokKind::RBrace | TokKind::RParen) {
                break;
            }
        }
    }
    // `import x from`.
    f.toks.get(t + 1).is_some_and(|n| n.kw == Kw::From)
}

/// Scans `paths` for server action uses and inline actions.
pub(crate) fn scan_actions(files: &Files<'_>, paths: &[PathBuf]) -> Vec<Site> {
    let mut out = Vec::new();
    let mut seen: HashSet<(PathBuf, u32, PathBuf, String)> = HashSet::new();
    let mut push =
        |out: &mut Vec<Site>, f: &JsFile, line: u32, via: String, target: (PathBuf, String)| {
            if !seen.insert((f.path.clone(), line, target.0.clone(), target.1.clone())) {
                return;
            }
            out.push(Site {
                file: f.path.clone(),
                line,
                kind: InvokeKind::Action,
                api: via.clone(),
                method: Some("POST".into()),
                targets: vec![SiteTarget::Action {
                    file: target.0,
                    export: target.1,
                }],
                via: vec![via],
            });
        };
    // The names an action can be imported under: an import of another name is not
    // one.
    let mut action_names: HashSet<String> = HashSet::new();
    let mut any_module = false;
    for p in paths {
        let Some(f) = files.get(p) else { continue };
        if is_action_module(&f) {
            any_module = true;
            action_names.extend(f.exports.keys().cloned());
            action_names.extend(f.reexports.iter().filter_map(|r| r.exported.clone()));
        }
    }
    let action_names = files.export_aliases(paths, action_names);
    for p in paths {
        let Some(f) = files.get(p) else { continue };
        // Inline actions: `async function save() { 'use server'; … }`.
        if !is_action_module(&f) && memmem(&f.src, b"use server") {
            for fd in &f.funcs {
                if fd.block
                    && f.kind(fd.body.0) == TokKind::Str
                    && f.str_value(fd.body.0).as_deref() == Some("use server")
                {
                    let name = fd
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("anonymous@{}", f.line(fd.start)));
                    push(
                        &mut out,
                        &f,
                        f.line(fd.start),
                        "inline".into(),
                        (f.path.clone(), format!("<inline:{name}>")),
                    );
                }
            }
        }
        // Imported actions.
        let mut names: Vec<(String, Rc<JsFile>, Option<String>)> = Vec::new();
        for imp in &f.imports {
            if !any_module {
                break;
            }
            match &imp.imported {
                Imported::Named(n) if !action_names.contains(n) => continue,
                Imported::Default if !action_names.contains("default") => continue,
                _ => {}
            }
            if imp.imported == Imported::Namespace {
                if let Some(g) = files.imported_namespace(&f, &imp.local)
                    && is_action_module(&g)
                {
                    names.push((imp.local.clone(), g, None));
                }
                continue;
            }
            let Some(path) = files.resolve(&f.path, &imp.module) else {
                continue;
            };
            let Some(first) = files.get(&path) else {
                continue;
            };
            let export = match &imp.imported {
                Imported::Named(n) => n.clone(),
                _ => "default".to_string(),
            };
            // The module the import names, or the module that declares the export behind a
            // barrel.
            if is_action_module(&first) {
                let (g, n) = files
                    .resolve_export(&first, &export)
                    .filter(|(g, _)| is_action_module(g))
                    .map(|(g, _)| (g, export.clone()))
                    .unwrap_or((first.clone(), export.clone()));
                names.push((imp.local.clone(), g, Some(n)));
            } else if let Some((g, local)) = files.resolve_export(&first, &export)
                && is_action_module(&g)
            {
                // The export name in the declaring module.
                let n = g
                    .exports
                    .iter()
                    .find(|(_, l)| **l == local)
                    .map(|(e, _)| e.clone())
                    .unwrap_or(local);
                names.push((imp.local.clone(), g, Some(n)));
            }
        }
        if names.is_empty() {
            continue;
        }
        let mut used: HashSet<usize> = HashSet::new();
        for t in 0..f.toks.len() {
            if f.kind(t) != TokKind::Ident {
                continue;
            }
            let text = f.text(t);
            let Some(i) = names.iter().position(|(n, _, _)| n == text) else {
                continue;
            };
            if not_a_use(&f, t) {
                continue;
            }
            // A shadowing declaration is another binding.
            if !matches!(
                f.lookup(text, t),
                Some(crate::nextjs::jseval::Decl::Import(_))
            ) {
                continue;
            }
            let (_, g, export) = &names[i];
            match export {
                Some(e) => {
                    used.insert(i);
                    push(
                        &mut out,
                        &f,
                        f.line(t),
                        how(&f, t),
                        (g.path.clone(), e.clone()),
                    );
                }
                None => {
                    // `actions.save(…)`.
                    if matches!(f.kind(t + 1), TokKind::Dot | TokKind::OptChain)
                        && f.kind(t + 2) == TokKind::Ident
                    {
                        used.insert(i);
                        let e = f.text(t + 2).to_string();
                        push(&mut out, &f, f.line(t), how(&f, t + 2), (g.path.clone(), e));
                    }
                }
            }
        }
        // Imported without a use in the file (handed on by a re-export).
        for (i, (local, g, export)) in names.iter().enumerate() {
            if used.contains(&i) {
                continue;
            }
            if let Some(e) = export {
                let line = f
                    .toks
                    .iter()
                    .position(|t| {
                        t.kind == TokKind::Ident
                            && std::str::from_utf8(t.text(&f.src)).ok() == Some(local.as_str())
                    })
                    .map_or(1, |t| f.line(t));
                push(
                    &mut out,
                    &f,
                    line,
                    "import".into(),
                    (g.path.clone(), e.clone()),
                );
            }
        }
    }
    out
}

fn memmem(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}
