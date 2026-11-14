// Class fields written through Object.assign and destructuring.
const fs = require('node:fs');
const path = require('node:path');
const K = (i: number) => process.env['FLOW_K' + i] === '1';
const fx = (...p: string[]) => path.join(__dirname, 'fx', ...p);
class Merged {
  private readonly file: string = 'me-1.json';
  merge(other: Merged) { Object.assign(other, { file: 'me-2.json' }); }
  read() { try { fs.readFileSync(fx(this.file)); } catch {} }
}
const me = new Merged();
me.merge(me);
me.read();
class Destructured {
  private a = 'ds-1.json';
  set() { [this.a] = ['ds-2.json']; }
  read() { try { fs.readFileSync(fx(this.a)); } catch {} }
}
const ds = new Destructured();
ds.set();
ds.read();
class Swapper {
  private current = 'sw-1.json';
  private next = 'sw-2.json';
  swap() { [this.current, this.next] = [this.next, this.current]; }
  read() { try { fs.readFileSync(fx(this.current)); } catch {} }
}
const sw = new Swapper();
sw.swap();
sw.read();
