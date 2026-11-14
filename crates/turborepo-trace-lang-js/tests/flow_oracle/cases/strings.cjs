// String method semantics.
const fs = require('fs');
const path = require('path');
const fx = (...p) => path.join(__dirname, 'fx', ...p);
const name = 'x.ts';
try { fs.readFileSync(fx(name.replace('.ts', '$&x'))); } catch {}
try { fs.readFileSync(fx('a.b'.replace('.', '$$'))); } catch {}
try { fs.readFileSync(fx('a.b'.replaceAll('.', "$'"))); } catch {}
try { fs.readFileSync(fx('﻿bom.json'.trim())); } catch {}
try { fs.readFileSync(fx('I.json'.toLowerCase())); } catch {}
try { fs.readFileSync(fx(['p', 'q'].join())); } catch {}
try { fs.readFileSync(fx(String(['p', 'q']))); } catch {}
try { fs.readFileSync(fx('' + ['r', 's'])); } catch {}
try { fs.readFileSync(fx(`${['t', 'u']}`)); } catch {}
try { fs.readFileSync(fx('x'.concat(1, 2))); } catch {}
try { fs.readFileSync(fx('abc'.replace('b', () => 'Z'))); } catch {}
try { fs.readFileSync(fx('abc'.replace('', '-'))); } catch {}
try { fs.readFileSync(fx('a/b'.split('/').join('_'))); } catch {}
try { fs.readFileSync(fx('  pad '.trimStart())); } catch {}
const obj = { toString() { return 'custom.json'; } };
try { fs.readFileSync(fx('' + obj)); } catch {}
try { fs.readFileSync(fx(`${obj}`)); } catch {}
try { fs.readFileSync(fx(String(obj))); } catch {}
const withValueOf = { valueOf() { return 'valueof.json'; } };
try { fs.readFileSync(fx('' + withValueOf)); } catch {}
const d = new Date(0);
try { fs.readFileSync(fx('y' + d.getUTCFullYear())); } catch {}
try { fs.readFileSync(fx('A'.toLowerCase() + '.json')); } catch {}
try { fs.readFileSync(fx('ab'.replace('a', 'b').replace('b', 'c'))); } catch {}
try { fs.readFileSync(fx('aaa'.replace('a', 'b'))); } catch {}
try { fs.readFileSync(fx('aaa'.replaceAll('a', 'b'))); } catch {}
try { fs.readFileSync(fx('ß.json'.toUpperCase())); } catch {}
