#!/usr/bin/env node
// Groups the findings of a report of oracle.mjs (--json FILE) for triage: per program with
// findings, the area of every missed file (the directory of the helper that loaded it), the
// sinks of the site files in that area and the bound dependencies the binder reported for
// them.
//
//   node triage.mjs target/xfile-oracle/report.json [--baseline] [--no-opaque]
//   node triage.mjs target/xfile-oracle/report.json --show NAME [--all]
//
// --no-opaque leaves out the areas with a sink whose value is `opaque(...)` (one finding
// class: the open record of a derived value lands on one supplier only).
// --show prints one program of the report: its findings, its sources (without the plain
// modules that are only loaded, unless --all), which entries loaded what, and the binder's
// lines.
import fs from 'node:fs';

const report = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const baseline = process.argv.includes('--baseline');
const show = process.argv.indexOf('--show');
if (show > 0) {
  const r = report.results.find((x) => x.name === process.argv[show + 1]);
  if (!r) throw new Error(`no program ${process.argv[show + 1]} in the report (only programs with findings are kept)`);
  console.log(`${r.name} [${r.shapes.join(', ')}]`);
  for (const f of r.findings) console.log(`FINDING ${f.entry} loads ${f.file}`);
  for (const f of r.baseline) console.log(`BASELINE ${f.entry} loads ${f.file}`);
  const plain = /^(module\.exports = \{ (name|dep): '[^']*' \};|exports\.name = '[^']*';|export const name = '[^']*';\nexport default \{ name \};)\n$/;
  for (const [f, text] of Object.entries(r.files ?? {}).sort()) {
    if (/(^|\/)(package\.json|jest\.config\.js|README\.md)$/.test(f)) continue;
    if (!process.argv.includes('--all') && (plain.test(text) || /^test\('\w+', \(\) => \{\n {2}expect\(1\)\.toBe\(1\);\n\}\);\n$/.test(text))) continue;
    console.log(`--- ${f}\n${text.replace(/^/gm, '  | ').replace(/\n {2}\| $/, '')}`);
  }
  for (const [f, es] of Object.entries(r.loaded ?? {}).sort()) if (!(es.length === 1 && es[0] === f)) console.log(`loaded ${f}: ${es.join(', ')}`);
  for (const l of r.bind ?? []) if (!l.startsWith('closed site')) console.log(`bind: ${l}`);
  for (const e of r.errors ?? []) console.log(`error: ${e}`);
  process.exit(0);
}
// pkgs/p1/mut3/mods/c.js -> pkgs/p1/mut3
const areaOf = (file) => /^(pkgs\/[^/]+\/[a-z]+\d+)\//.exec(file)?.[1] ?? file.split('/').slice(0, 2).join('/');

const groups = new Map();
for (const r of report.results) {
  const list = baseline ? r.baseline : r.findings;
  if (!list.length) continue;
  const areas = new Map();
  for (const f of list) {
    const a = areaOf(f.file);
    if (!areas.has(a)) areas.set(a, new Set());
    areas.get(a).add(`${f.entry} -> ${f.file}`);
  }
  // The facts sections of the binder's debug output, by site file.
  const sections = new Map();
  let current = null;
  for (const line of r.bind ?? []) {
    const m = /^== facts of (.*)$/.exec(line);
    if (m) {
      current = m[1];
      sections.set(current, []);
    } else if (/^(dep|closed pkgs)/.test(line)) {
      const site = /<- (\S+?):\d+$/.exec(line)?.[1] ?? /^closed (\S+)/.exec(line)?.[1];
      if (site) {
        if (!sections.has(site)) sections.set(site, []);
        sections.get(site).push(line);
      }
    } else if (current) sections.get(current).push(line);
  }
  for (const [area, pairs] of areas) {
    // (A file outside every helper's directory, a package's index file for example: the
    // sinks of the whole program.)
    let sinks = [...sections].filter(([file]) => file.startsWith(`${area}/`));
    if (!sinks.some(([, ls]) => ls.some((l) => l.startsWith('sink ')))) sinks = [...sections];
    const sinkLines = sinks.flatMap(([, ls]) => ls.filter((l) => l.startsWith('sink ')).map((l) => l.replace(/^sink \w+:\d+ /, '')));
    if (process.argv.includes('--no-opaque') && sinkLines.some((l) => l.includes('opaque('))) continue;
    const key = sinkLines.length ? sinkLines.map((l) => l.replace(/#\d+/g, '#N')).sort().join(' ; ') : '(no sink in the area)';
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push({ name: r.name, shapes: r.shapes, area, pairs: [...pairs], sinks });
  }
}
for (const [key, items] of [...groups].sort((a, b) => b[1].length - a[1].length)) {
  console.log(`\n=== ${items.length}x sink: ${key}`);
  for (const it of items) {
    console.log(`  ${it.name} [${it.shapes.join(', ')}] ${it.area}: ${it.pairs.length} pairs, e.g. ${it.pairs[0]}`);
    for (const [file, ls] of it.sinks) for (const l of ls) if (!l.startsWith('closed site')) console.log(`      ${file.slice(it.area.length + 1)}: ${l.slice(0, 150)}`);
  }
}
