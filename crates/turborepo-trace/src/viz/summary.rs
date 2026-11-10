//! `affected --format summary`: the answers of the HTML report as aligned text,
//! for a terminal or a CI log. Bars are block characters on one scale per
//! section; colour is used only on a terminal.

use std::io::{self, Write};

use super::Model;

/// Rows per section.
const TOP: usize = 8;

/// `1234567` as `1,234,567`.
fn num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn pct(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "0%".to_owned();
    }
    let p = 100.0 * part as f64 / whole as f64;
    if p > 0.0 && p < 0.1 {
        "<0.1%".to_owned()
    } else {
        format!("{p:.1}%")
    }
}

/// A bar of `value` on a scale where `max` fills `width` cells, in eighths of a
/// cell.
fn bar(value: usize, max: usize, width: usize) -> String {
    if max == 0 || value == 0 || width == 0 {
        return String::new();
    }
    let eighths = ((value as f64 / max as f64) * (width * 8) as f64).round() as usize;
    let eighths = eighths.clamp(1, width * 8);
    let mut s = "█".repeat(eighths / 8);
    let rest = eighths % 8;
    if rest > 0 {
        s.push(['▏', '▎', '▍', '▌', '▋', '▊', '▉'][rest - 1]);
    }
    s
}

/// `path` cut in the middle to `width` characters.
fn fit(path: &str, width: usize) -> String {
    let n = path.chars().count();
    if n <= width || width < 8 {
        return path.to_owned();
    }
    let keep = width - 1;
    let head = keep / 3;
    let tail = keep - head;
    let a: String = path.chars().take(head).collect();
    let b: String = path.chars().skip(n - tail).collect();
    format!("{a}…{b}")
}

struct Style {
    tty: bool,
}

impl Style {
    fn head(&self, s: &str) -> String {
        if self.tty {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    }
    fn dim(&self, s: &str) -> String {
        if self.tty {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    }
}

/// `text` wrapped at word boundaries to `room` columns.
fn wrap(text: &str, room: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > room {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A dimmed note, wrapped to `width` columns, indented by two.
fn note(out: &mut dyn Write, st: &Style, width: usize, text: &str) -> io::Result<()> {
    for line in wrap(text, width.saturating_sub(2).max(20)) {
        writeln!(out, "  {}", st.dim(&line))?;
    }
    Ok(())
}

/// A sentence, wrapped to `width` columns, indented by two.
fn para(out: &mut dyn Write, width: usize, text: &str) -> io::Result<()> {
    for line in wrap(text, width.saturating_sub(2).max(20)) {
        writeln!(out, "  {line}")?;
    }
    Ok(())
}

/// Writes the report, at most `width` columns wide.
pub(super) fn write(m: &Model, out: &mut dyn Write, width: usize, tty: bool) -> io::Result<()> {
    let st = Style { tty };
    let t = &m.totals;
    let chosen = t.selected + t.unknown;
    let since = match &m.meta.since {
        // A full commit id is cut like `git log --abbrev-commit`.
        Some(rev) if rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()) => {
            format!(" since {}", &rev[..12])
        }
        Some(rev) => format!(" since {rev}"),
        None => String::new(),
    };
    let headline = format!(
        "{} of {} entries selected ({}) by {} change(s){since}",
        num(chosen),
        num(t.entries),
        pct(chosen, t.entries),
        num(t.changes),
    );
    for line in wrap(&headline, width) {
        writeln!(out, "{}", st.head(&line))?;
    }
    let track = width.saturating_sub(4).min(60);
    let filled = bar(chosen, t.entries.max(1), track);
    let cells = filled.chars().count();
    writeln!(
        out,
        "  {filled}{}",
        st.dim(&"░".repeat(track - cells.min(track)))
    )?;
    if t.unknown > 0 {
        para(
            out,
            width,
            &format!(
                "{} affected, {} unknown (uncertain dependencies)",
                num(t.selected),
                num(t.unknown)
            ),
        )?;
    }
    if m.meta.steering_only {
        para(
            out,
            width,
            "only repository steering files changed: uncertainty outside them does not count",
        )?;
    }

    // By kind.
    let kinds: Vec<&super::KindRow> = m.kinds.iter().filter(|k| k.total > 0).collect();
    if kinds.len() > 1 || kinds.iter().any(|k| k.id != "file") {
        writeln!(out, "\n{}", st.head("How much, by kind"))?;
        let w = kinds.iter().map(|k| k.label.len()).max().unwrap_or(0);
        let max = kinds.iter().map(|k| k.total).max().unwrap_or(0);
        let (ws, wt) = (
            kinds
                .iter()
                .map(|k| num(k.selected).len())
                .max()
                .unwrap_or(1),
            kinds.iter().map(|k| num(k.total).len()).max().unwrap_or(1),
        );
        let lane = width.saturating_sub(w + ws + wt + 20).min(40);
        for k in &kinds {
            writeln!(
                out,
                "  {:<w$}  {:>ws$} of {:>wt$}  {:>6}  {}",
                k.label,
                num(k.selected),
                num(k.total),
                pct(k.selected, k.total),
                bar(k.selected, max, lane),
            )?;
        }
    }

    // By first cause.
    let causes: Vec<_> = m.causes.iter().filter(|c| c.first > 0).collect();
    if !causes.is_empty() {
        writeln!(
            out,
            "\n{}",
            st.head("Why: the most direct cause of each selected entry")
        )?;
        let w = causes.iter().map(|c| c.label.len()).max().unwrap_or(0);
        let wn = causes.iter().map(|c| num(c.first).len()).max().unwrap_or(1);
        let max = causes.iter().map(|c| c.first).max().unwrap_or(0);
        let lane = width.saturating_sub(w + wn + 14).min(40);
        for c in &causes {
            writeln!(
                out,
                "  {:<w$}  {:>wn$}  {:>6}  {}",
                c.label,
                num(c.first),
                pct(c.first, chosen),
                bar(c.first, max, lane),
            )?;
        }
        if t.by_change + t.by_floor + t.by_named > 0 {
            note(
                out,
                &st,
                width,
                &format!(
                    "a change reaches {} ({} only a change), the floor {} ({} only the floor), a \
                     name {}",
                    num(t.by_change),
                    num(t.only_change),
                    num(t.by_floor),
                    num(t.only_floor),
                    num(t.by_named),
                ),
            )?;
        }
    }

    // From where: sources by the entries drawn from them.
    let mut drawn = vec![0usize; m.sources.len()];
    let mut held = vec![0usize; m.hubs.len()];
    for e in &m.entries {
        if e.6 >= 0 {
            drawn[e.6 as usize] += 1;
        }
        if e.14 >= 0 {
            held[e.14 as usize] += 1;
        }
    }
    let mut order: Vec<usize> = (0..m.sources.len()).filter(|&i| drawn[i] > 0).collect();
    order.sort_by(|&a, &b| {
        drawn[b]
            .cmp(&drawn[a])
            .then_with(|| m.sources[a].label.cmp(&m.sources[b].label))
    });
    if !order.is_empty() {
        writeln!(
            out,
            "\n{}",
            st.head("From where: what the selected entries are traced back to")
        )?;
        note(
            out,
            &st,
            width,
            "entries: drawn from it, each entry once; only: not selected without it; reaches: \
             every entry it reaches",
        )?;
        writeln!(
            out,
            "  {:>8} {:>8} {:>8}  source",
            "entries", "only", "reaches"
        )?;
        let lane = width.saturating_sub(31).max(20);
        for &i in order.iter().take(TOP) {
            let s = &m.sources[i];
            let kind = match s.kind {
                "site" => "open uncertainty in ",
                "name" => "named by ",
                "package" => "package ",
                _ => "",
            };
            writeln!(
                out,
                "  {:>8} {:>8} {:>8}  {}",
                num(drawn[i]),
                num(s.today as usize),
                s.inclusive.map_or("-".to_owned(), |n| num(n as usize)),
                fit(&format!("{kind}{}", s.label), lane),
            )?;
        }
        if order.len() > TOP {
            let rest: usize = order.iter().skip(TOP).map(|&i| drawn[i]).sum();
            writeln!(
                out,
                "  {}",
                st.dim(&format!(
                    "… {} more source(s) with {} entries",
                    num(order.len() - TOP),
                    num(rest)
                ))
            )?;
        }
        let idle = m
            .changes
            .iter()
            .filter(|c| c.source.is_none_or(|s| drawn[s as usize] == 0))
            .count();
        if idle > 0 {
            let mut by: Vec<(&str, usize)> = Vec::new();
            for c in m
                .changes
                .iter()
                .filter(|c| c.source.is_none_or(|s| drawn[s as usize] == 0))
            {
                let label = match c.status {
                    "steering" => "steering files",
                    "lockfile" => "lockfiles compared by package",
                    "config" => "configs applied by field",
                    "workflow" => "workflows read by job",
                    "erased" => "with unchanged JavaScript",
                    "barrel" => "behind a barrel",
                    "unmatched" => "nothing depends on",
                    _ => "no entry depends on",
                };
                match by.iter_mut().find(|(l, _)| *l == label) {
                    Some(x) => x.1 += 1,
                    None => by.push((label, 1)),
                }
            }
            let parts: Vec<String> = by.iter().map(|(l, n)| format!("{n} {l}")).collect();
            note(
                out,
                &st,
                width,
                &format!(
                    "{} of {} changed path(s) are the first cause of no entry: {}",
                    num(idle),
                    num(m.changes.len()),
                    parts.join(", ")
                ),
            )?;
        }
    }

    // Through what.
    let mut hubs: Vec<usize> = (0..m.hubs.len()).filter(|&i| held[i] > 0).collect();
    hubs.sort_by(|&a, &b| {
        held[b]
            .cmp(&held[a])
            .then_with(|| m.nodes[m.hubs[a].node as usize].cmp(&m.nodes[m.hubs[b].node as usize]))
    });
    if !hubs.is_empty() {
        writeln!(
            out,
            "\n{}",
            st.head("Through what: the hubs between the changes and the entries")
        )?;
        note(
            out,
            &st,
            width,
            "entries: their chain crosses it; only: every path to them crosses it",
        )?;
        writeln!(out, "  {:>8} {:>8}  hub", "entries", "only")?;
        let lane = width.saturating_sub(22).max(20);
        for &i in hubs.iter().take(TOP) {
            let h = &m.hubs[i];
            let via = if h.mech == "floor" { " (floor)" } else { "" };
            writeln!(
                out,
                "  {:>8} {:>8}  {}",
                num(held[i]),
                num(h.own as usize),
                fit(&format!("{}{via}", m.nodes[h.node as usize]), lane),
            )?;
        }
        if hubs.len() > TOP {
            writeln!(
                out,
                "  {}",
                st.dim(&format!("… {} more hub(s)", num(hubs.len() - TOP)))
            )?;
        }
    }

    // To where.
    let mut groups: Vec<&super::GroupRow> = m.groups.iter().filter(|g| g.selected > 0).collect();
    groups.sort_by(|a, b| {
        b.selected
            .cmp(&a.selected)
            .then_with(|| a.path.cmp(&b.path))
    });
    if !groups.is_empty() {
        writeln!(
            out,
            "\n{}",
            st.head("To where: selected entries by package")
        )?;
        let max = groups.iter().map(|g| g.total).max().unwrap_or(0);
        let wn = groups
            .iter()
            .take(TOP)
            .map(|g| num(g.selected).len())
            .max()
            .unwrap_or(1);
        let wt = groups
            .iter()
            .take(TOP)
            .map(|g| num(g.total).len())
            .max()
            .unwrap_or(1);
        let wp = groups
            .iter()
            .take(TOP)
            .map(|g| g.path.chars().count())
            .max()
            .unwrap_or(1)
            .min(width.saturating_sub(wn + wt + 30).max(16));
        let lane = width.saturating_sub(wp + wn + wt + 12).min(30);
        for g in groups.iter().take(TOP) {
            writeln!(
                out,
                "  {:>wn$} of {:>wt$}  {:<wp$}  {}",
                num(g.selected),
                num(g.total),
                fit(&g.path, wp),
                bar(g.selected, max, lane),
            )?;
        }
        if groups.len() > TOP {
            let rest: usize = groups.iter().skip(TOP).map(|g| g.selected).sum();
            writeln!(
                out,
                "  {}",
                st.dim(&format!(
                    "… {} more package(s) with {} entries",
                    num(groups.len() - TOP),
                    num(rest)
                ))
            )?;
        }
    }

    // The floor.
    let floor = m.floor.get("floor").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let total = m.floor.get("entries").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    writeln!(out, "\n{}", st.head("What keeps the floor"))?;
    if floor == 0 {
        para(
            out,
            width,
            "no entry reaches open uncertainty: nothing is selected on every change",
        )?;
    } else {
        para(
            out,
            width,
            &format!(
                "{} of {} entries ({}) reach open uncertainty and are selected on every change{}",
                num(floor),
                num(total),
                pct(floor, total),
                if m.meta.floor_applies {
                    ""
                } else {
                    " (not applied to this run)"
                }
            ),
        )?;
        let greedy = m.floor.get("greedy").and_then(|v| v.as_array());
        if let Some(rows) = greedy.filter(|r| !r.is_empty()) {
            note(
                out,
                &st,
                width,
                "in the order to resolve them: freed leave the floor with the site, left stay on \
                 it",
            )?;
            writeln!(out, "  {:>8} {:>8}  site", "freed", "left")?;
            let lane = width.saturating_sub(22).max(20);
            for r in rows.iter().take(TOP) {
                let get = |k: &str| r.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                writeln!(
                    out,
                    "  {:>8} {:>8}  {}",
                    num(get("freed")),
                    num(get("remaining")),
                    fit(r.get("path").and_then(|v| v.as_str()).unwrap_or(""), lane),
                )?;
            }
            note(
                out,
                &st,
                width,
                "`turbotrace uncertainty` lists the records of each site",
            )?;
        }
    }

    // Not selected.
    writeln!(out, "\n{}", st.head("What was not selected"))?;
    para(
        out,
        width,
        &format!(
            "{} of {} entries ({}): no change reaches them{}",
            num(t.unaffected),
            num(t.entries),
            pct(t.unaffected, t.entries),
            if floor > 0 && m.meta.floor_applies {
                " and they reach no open uncertainty"
            } else {
                ""
            }
        ),
    )?;
    let p = &m.pruning;
    if p.erased + p.barrels + p.type_only > 0 {
        let mut parts: Vec<String> = Vec::new();
        if p.erased > 0 {
            parts.push(format!("{} with unchanged JavaScript", num(p.erased)));
        }
        if p.barrels > 0 {
            parts.push(format!("{} behind a barrel", num(p.barrels)));
        }
        let noted = m
            .entries
            .iter()
            .filter(|e| e.3 == 0 && !e.13.is_empty())
            .count();
        if !parts.is_empty() {
            para(
                out,
                width,
                &format!(
                    "{} entr(ies) reach a changed file that was skipped for them: {}",
                    num(noted),
                    parts.join(", ")
                ),
            )?;
        }
        if p.type_only > 0 {
            para(
                out,
                width,
                &format!(
                    "{} entr(ies) reach a change only through type-only imports",
                    num(p.type_only)
                ),
            )?;
        }
    }
    let mut rest: Vec<&super::GroupRow> =
        m.groups.iter().filter(|g| g.total > g.selected).collect();
    rest.sort_by(|a, b| {
        (b.total - b.selected)
            .cmp(&(a.total - a.selected))
            .then_with(|| a.path.cmp(&b.path))
    });
    for g in rest.iter().take(TOP.min(5)) {
        writeln!(
            out,
            "  {:>8}  {}",
            num(g.total - g.selected),
            fit(&g.path, width.saturating_sub(12).max(20))
        )?;
    }
    if rest.len() > 5 {
        writeln!(
            out,
            "  {}",
            st.dim(&format!("… {} more package(s)", num(rest.len() - 5)))
        )?;
    }
    writeln!(out)?;
    note(
        out,
        &st,
        width,
        "--explain <entry> prints the chain of one entry; --html writes the full report",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_bars() {
        assert_eq!(num(0), "0");
        assert_eq!(num(999), "999");
        assert_eq!(num(1000), "1,000");
        assert_eq!(num(1234567), "1,234,567");
        assert_eq!(pct(1, 3), "33.3%");
        assert_eq!(pct(1, 5000), "<0.1%");
        assert_eq!(pct(0, 0), "0%");
        assert_eq!(bar(10, 10, 4), "████");
        assert_eq!(bar(5, 10, 4), "██");
        assert_eq!(bar(1, 1000, 4), "▏");
        assert_eq!(bar(0, 10, 4), "");
        assert_eq!(bar(9, 16, 2), "█▏");
    }

    #[test]
    fn fits() {
        assert_eq!(fit("short.ts", 20), "short.ts");
        let f = fit("packages/very/long/path/to/a/file.test.ts", 20);
        assert_eq!(f.chars().count(), 20);
        assert!(f.starts_with("packag") && f.ends_with("file.test.ts"));
    }
}
