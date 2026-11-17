// path.* with segments that are only partly known.
const fs = require('fs');
const path = require('path');
const nested = process.env.FLOW_NESTED;
const q1 = path.join(__dirname, 'fx', nested, '..', 'sib.json');
try { fs.readFileSync(q1); } catch {}
const q2 = path.resolve(__dirname, 'fx', process.env.FLOW_ABS);
try { fs.readFileSync(q2); } catch {} /* assumed: an unknown segment of path.resolve is relative */
const q3 = path.join(__dirname, 'fx', path.basename(nested));
try { fs.readFileSync(q3); } catch {}
const q4 = path.join(__dirname, 'fx', path.dirname(nested), 'x.json');
try { fs.readFileSync(q4); } catch {}
const q5 = path.join(__dirname, 'fx', nested + '/../../up.json');
try { fs.readFileSync(q5); } catch {}
for (const e of fs.readdirSync(path.join(__dirname, 'fx'), { recursive: true })) { try { fs.statSync(path.join(__dirname, 'fx', e)); } catch {} }
for (const e of fs.readdirSync(path.join(__dirname, 'fx'), { withFileTypes: true, recursive: true })) { try { fs.statSync(path.join(e.parentPath, e.name)); } catch {} }
const q6 = path.join(__dirname, 'fx', path.extname('a.json'));
try { fs.readFileSync(q6); } catch {}
const q7 = path.join(__dirname, 'fx', path.relative('/a/b', '/a/c/d.json'));
try { fs.readFileSync(q7); } catch {}
const q8 = path.join(__dirname, 'fx', path.parse('/q/name.ext').name + '.json');
try { fs.readFileSync(q8); } catch {}
const q9 = path.resolve('fx', '/abs-reset', 'a.json');
try { fs.readFileSync(q9); } catch {}
const q10 = path.join(__dirname, 'fx', path.basename(nested, '2'));
try { fs.readFileSync(q10); } catch {}
const q11 = path.join(__dirname, 'fx/sub', '..', '..', 'fx', path.sep, 'a.json');
try { fs.readFileSync(q11); } catch {}
const q12 = path.dirname(path.dirname(path.join(__dirname, 'fx', nested, 'leaf.json')));
try { fs.readdirSync(q12); } catch {}
const q13 = path.join(__dirname, 'fx', nested).replace('n1', 'm1');
try { fs.readFileSync(q13); } catch {}
const q14 = path.join(__dirname, 'fx', 'a.json').replace('fx', 'gx');
try { fs.readFileSync(q14); } catch {}
const q15 = path.join(__dirname, 'fx').slice(0, -2) + 'hx/a.json';
try { fs.readFileSync(q15); } catch {}
const q16 = __filename.replace('.cjs', '.json');
try { fs.readFileSync(q16); } catch {}
const q17 = __dirname.replace('src', 'lib') + '/a.json';
try { fs.readFileSync(q17); } catch {}
const q18 = path.join(__dirname, '..', path.basename(__dirname) + '-extra', 'a.json');
try { fs.readFileSync(q18); } catch {}
const q19 = path.dirname(path.join(__dirname, 'fx', nested));
try { fs.readdirSync(q19); } catch {}
const q20 = path.join(path.join(__dirname, 'fx', nested), '..', '..', 'top.json');
try { fs.readFileSync(q20); } catch {}
const q21 = path.join(__dirname, 'fx', 'pre-' + nested, '..', 'sib2.json');
try { fs.readFileSync(q21); } catch {}
const q22 = path.normalize(__dirname + '/fx/' + nested + '/../norm.json');
try { fs.readFileSync(q22); } catch {}
const q23 = path.resolve(__dirname, 'fx', nested, '../res.json');
try { fs.readFileSync(q23); } catch {}
