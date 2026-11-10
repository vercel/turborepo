// Promise shapes and file handles.
const fs = require('fs');
const path = require('path');
const fx = (...p) => path.join(__dirname, 'fx', ...p);
Promise.reject(new Error('x')).then(() => 'ok.json', () => 'rejected.json').then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
Promise.resolve('p1.json').finally(() => 'ignored.json').then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
Promise.all(['pa.json', Promise.resolve('pb.json')]).then(([x, y]) => { try { fs.readFileSync(fx(x)); } catch {} try { fs.readFileSync(fx(y)); } catch {} });
(async () => { const fh = await fs.promises.open(fx('a.json')); const data = await fh.readFile('utf8'); try { require('./fx/' + JSON.parse(data).file); } catch {} })();
(async () => { try { await Promise.reject('thrown-async.json'); } catch (e) { try { fs.readFileSync(fx(e)); } catch {} } })();
async function later() { return 'later.json'; }
later().then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
(async () => { for await (const f of ['fa1.json', Promise.resolve('fa2.json')]) { try { fs.readFileSync(fx(f)); } catch {} } })();
const st = fs.statSync(fx('a.json'));
try { fs.readFileSync(fx('size-' + st.size)); } catch {}
Promise.race(['race1.json', 'race2.json']).then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
Promise.allSettled([Promise.resolve('settled.json')]).then((rs) => { try { fs.readFileSync(fx(rs[0].value)); } catch {} });
new Promise((res) => res('constructed.json')).then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
Promise.reject('caught.json').catch((e) => e).then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
Promise.resolve().then(() => 'chained.json').then((f) => { try { fs.readFileSync(fx(f)); } catch {} });
(async () => { const data = fs.readFileSync(fx('a.json'), 'utf8'); try { fs.readFileSync(fx(JSON.parse(data).file)); } catch {} })();
(async () => { const fh = await fs.promises.open(fx('a.json')); const data = await fh.readFile('utf8'); try { fs.readFileSync(fx(JSON.parse(data).file)); } catch {} })();
