// ESM: import.meta, URL, dynamic import.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const here = path.dirname(fileURLToPath(import.meta.url));
try { fs.readFileSync(new URL('./fx/a.json', import.meta.url)); } catch {}
try { fs.readFileSync(new URL('./fx/a.json?raw', import.meta.url)); } catch {}
const nested = process.env.FLOW_NESTED;
try { fs.readFileSync(new URL(`./fx/${nested}/../sib.json`, import.meta.url)); } catch {}
try { fs.readFileSync(new URL(process.env.FLOW_ABS, import.meta.url)); } catch {} /* assumed: an unknown URL input is relative */
try { fs.readFileSync(path.join(here, 'fx', 'b.json')); } catch {}
try { fs.readFileSync(path.join(import.meta.dirname, 'fx', 'b.json')); } catch {}
const mods = ['./fx/m1.mjs', './fx/m2.mjs'];
for (const m of mods) { await import(m).catch(() => {}); }
await import('./fx/m3.mjs?t=' + Date.now()).catch(() => {});
await import(new URL('./fx/m4.mjs', import.meta.url).href).catch(() => {});
await import(`./fx/${nested}.mjs`).catch(() => {});
const base = new URL('./fx/', import.meta.url);
try { fs.readFileSync(new URL('sub/c.json', base)); } catch {}
try { fs.readFileSync(new URL('../outside.json', base)); } catch {}
try { fs.readFileSync(fileURLToPath(new URL('fx/a.json', import.meta.url))); } catch {}
try { fs.readFileSync(new URL('fx/a%20b.json', import.meta.url).pathname); } catch {}
export let current = './fx/cur1.mjs';
export function setCurrent(v) { current = v; }
setCurrent('./fx/cur2.mjs');
await import(current).catch(() => {});
const page = 1;
await import(`./fx/page${page + 1}.mjs`).catch(() => {});
const langs = { true: './fx/yes.mjs', false: './fx/no.mjs' };
await import(langs[process.env.FLOW_K0 === '1']).catch(() => {});
