// What the token evaluator (tier 1) claims on its own: names it reads without knowing
// scopes, top-level bindings that change, numbers. Run with --flow-off or --decline.
const fs = require('fs');
const path = require('path');
const K = (i) => process.env['FLOW_K' + i] === '1';
const dir = 'top';
function byParam(dir) { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'a.json')); } catch {} }
byParam('param');
function byLocal() { const dir = 'local'; try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'b.json')); } catch {} }
byLocal();
{ const dir = 'block'; try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'c.json')); } catch {} }
for (const dir of ['loop1', 'loop2']) { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'd.json')); } catch {} }
const byArrow = (dir) => { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'e.json')); } catch {} };
byArrow('arrow');
try { throw 'caught'; } catch (dir) { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'f.json')); } catch {} }
class C { m(dir) { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'g.json')); } catch {} } }
new C().m('method');
function byDestructuring({ dir }) { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'h.json')); } catch {} }
byDestructuring({ dir: 'destructured' });
function byDefault(dir = 'default') { try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'i.json')); } catch {} }
byDefault();
function byVar() { var dir = 'var'; try { fs.readFileSync(path.join(__dirname, 'fx', dir, 'j.json')); } catch {} }
byVar();
function byInner() { function dir() {} try { fs.readFileSync(path.join(__dirname, 'fx', String(dir.name), 'k.json')); } catch {} }
byInner();
const viaTemplate = (dir) => { try { fs.readFileSync(`${__dirname}/fx/${dir}/l.json`); } catch {} };
viaTemplate('templated');
const viaRequire = (dir) => { try { require('./fx/' + dir + '/m.js'); } catch {} };
viaRequire('required');
let mode = 'm1';
mode = 'm2';
try { fs.readFileSync(path.join(__dirname, 'fx', mode)); } catch {}
var kind = 'k1';
function setKind() { kind = 'k2'; }
setKind();
try { fs.readFileSync(path.join(__dirname, 'fx', kind)); } catch {}
let late;
late = 'l1';
try { fs.readFileSync(path.join(__dirname, 'fx', late)); } catch {}
let cond = 'c1';
if (K(0)) cond = 'c2';
try { fs.readFileSync(path.join(__dirname, 'fx', cond)); } catch {}
let grown = 'g';
grown += '-more';
try { fs.readFileSync(path.join(__dirname, 'fx', grown)); } catch {}
const parts = ['p1'];
parts.push('p2');
try { fs.readFileSync(path.join(__dirname, 'fx', parts.join('/'))); } catch {}
const pair = ['q1', 'q2'];
pair[1] = 'q3';
try { fs.readFileSync(path.join(__dirname, 'fx', pair.join('/'))); } catch {}
const n = 1;
try { fs.readFileSync(path.join(__dirname, 'fx', 'f' + (n + 1) + '.json')); } catch {}
try { fs.readFileSync(path.join(__dirname, 'fx', 'g' + (n + n) + '.json')); } catch {}
try { fs.readFileSync(path.join(__dirname, 'fx', `h${n + 1}.json`)); } catch {}
const flag = K(1);
try { fs.readFileSync(path.join(__dirname, 'fx', 'x' + (flag && '-on') + '.json')); } catch {}
const empty = '';
try { fs.readdirSync(path.join(__dirname, 'fx', empty && 'nope')); } catch {}
try { fs.readFileSync([__dirname, 'fx', K(2) ? 'sub' : undefined, 'c.json'].join('/')); } catch {}
const name = 'x.ts';
try { fs.readFileSync(path.join(__dirname, 'fx', name.replace('.ts', '.js'))); } catch {}
try { fs.readFileSync(path.join(__dirname, 'fx', name.toUpperCase())); } catch {}
try { fs.readFileSync(path.join(__dirname, 'fx', name.concat('.bak'))); } catch {}
try { fs.readFileSync(path.join(__dirname, 'fx', name.toString())); } catch {}
function shadowPath(path) { try { fs.readFileSync(path.join(__dirname, 'fx', 'n.json')); } catch {} }
shadowPath({ join: (...a) => a[0] + '/elsewhere/' + a[2] });
function shadowDirname(__dirname) { try { fs.readFileSync(path.join(__dirname, 'fx', 'o.json')); } catch {} }
shadowDirname(path.join(__dirname, 'other'));
const base = path.join(__dirname, 'fx');
function shadowBase(base) { try { fs.readFileSync(path.join(base, 'p.json')); } catch {} }
shadowBase(path.join(__dirname, 'fx', 'deeper'));
const viaObject = { dir: 'objdir' };
try { fs.readFileSync(path.join(__dirname, 'fx', viaObject.dir, 'q.json')); } catch {}
const first = ['r1.json', 'r2.json'][K(3) ? 0 : 1];
try { fs.readFileSync(path.join(__dirname, 'fx', first)); } catch {}
const hoisted = later();
function later() { return 's.json'; }
try { fs.readFileSync(path.join(__dirname, 'fx', hoisted)); } catch {}
const ternary = K(0) ? 't1.json' : 't2.json';
try { fs.readFileSync(path.join(__dirname, 'fx', ternary)); } catch {}
const either = process.env.FLOW_UNSET || 'u.json';
try { fs.readFileSync(path.join(__dirname, 'fx', either)); } catch {}
const present = process.env.FLOW_PRESENT || 'v.json';
try { fs.readFileSync(path.join(__dirname, 'fx', present)); } catch {}
