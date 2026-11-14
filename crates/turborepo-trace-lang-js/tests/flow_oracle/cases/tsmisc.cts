// TypeScript-only constructs: enums, namespaces, overloads, `as const`.
const fs = require('node:fs');
const path = require('node:path');
const K = (i: number) => process.env['FLOW_K' + i] === '1';
const fx = (...p: string[]) => path.join(__dirname, 'fx', ...p);
enum Mode { Dev = 'dev', Prod = 'prod' }
const mode: Mode = K(0) ? Mode.Dev : Mode.Prod;
try { fs.readFileSync(fx(`mode-${mode}.json`)); } catch {}
enum Num { Zero, One, Two }
try { fs.readFileSync(fx(`num-${Num.Two}.json`)); } catch {}
try { fs.readFileSync(fx(`numname-${Num[1]}.json`)); } catch {}
const TABLE = ['ro-1.json', 'ro-2.json'] as const;
module.exports.TABLE = TABLE;
for (const f of TABLE) { try { fs.readFileSync(fx(f)); } catch {} }
namespace NS { export const file = 'ns.json'; }
try { fs.readFileSync(fx(NS.file)); } catch {}
function over(x: string): void;
function over(x: number): void;
function over(x: any) { try { fs.readFileSync(fx(String(x))); } catch {} }
over('ov-1.json');
over(7);
