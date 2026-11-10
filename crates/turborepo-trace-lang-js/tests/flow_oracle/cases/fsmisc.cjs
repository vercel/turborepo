// Temporary directories, URLs and query strings.
const fs = require('fs');
const path = require('path');
const os = require('os');
const url = require('url');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'case-'));
try { fs.readFileSync(path.join(tmp, 'in-tmp.json')); } catch {}
const inRepo = fs.mkdtempSync(path.join(__dirname, 'fx', 'mk-'));
try { fs.readFileSync(path.join(inRepo, 'made.json')); } catch {}
const maybeTmp = process.env.FLOW_UNSET_TMP || path.join(__dirname, 'fx');
try { fs.readFileSync(path.join(maybeTmp, 'a.json')); } catch {}
const real = fs.realpathSync(path.join(__dirname, 'fx'));
try { fs.readFileSync(path.join(real, 'b.json')); } catch {}
try { import('./fx/mod.mjs?v=' + 1).catch(() => {}); } catch {}
try { fs.readFileSync(new URL('./fx/a.json?raw', url.pathToFileURL(__filename))); } catch {}
try { fs.readFileSync(new URL('./fx/sp%61ce.json', url.pathToFileURL(__filename))); } catch {}
try { fs.readFileSync(url.fileURLToPath(new URL('fx/b.json#frag', url.pathToFileURL(__dirname + '/')))); } catch {}
const escaped = path.join(os.tmpdir(), '..', 'proj', 'pkg', 'src', 'fx', 'a.json');
try { fs.readFileSync(escaped); } catch {} /* assumed: a path below the temporary directory stays there */
const tmpOrRepo = [os.tmpdir(), path.join(__dirname, 'fx')];
for (const d of tmpOrRepo) { try { fs.readFileSync(path.join(d, 'either.json')); } catch {} }
const rel = path.relative(os.tmpdir(), path.join(__dirname, 'fx', 'a.json'));
try { fs.readFileSync(path.resolve(os.tmpdir(), rel)); } catch {} /* assumed: a path below the temporary directory stays there */
const backToRepo = path.resolve(tmp, __dirname, 'fx', 'b.json');
try { fs.readFileSync(backToRepo); } catch {}
