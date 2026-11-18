// Class fields: writes through other references, subclass overrides, updates.
const fs = require('node:fs');
const path = require('node:path');
const K = (i: number) => process.env['FLOW_K' + i] === '1';
const fx = (...p: string[]) => path.join(__dirname, 'fx', ...p);
class A {
  private dir = 'a-dir';
  static make() { const a = new A(); a.dir = 'b-dir'; return a; }
  read() { try { fs.readFileSync(fx(this.dir, 'f.json')); } catch {} }
}
A.make().read();
class Base {
  protected readonly name: string = 'base';
  load() { try { fs.readFileSync(fx(this.name + '.json')); } catch {} }
}
class Derived extends Base {
  protected readonly name: string = 'derived';
}
new Base().load();
new Derived().load();
class Counter {
  private n = 0;
  next() { this.n++; try { fs.readFileSync(fx('c' + this.n + '.json')); } catch {} }
}
const counter = new Counter();
counter.next();
counter.next();
class Aliased {
  private file = 'al-1.json';
  swap() { const self = this; self.file = 'al-2.json'; }
  read() { try { fs.readFileSync(fx(this.file)); } catch {} }
}
const al = new Aliased();
al.swap();
al.read();
class Casted {
  private file = 'ca-1.json';
  read() { try { fs.readFileSync(fx(this.file)); } catch {} }
}
const ca = new Casted();
(ca as any).file = 'ca-2.json';
ca.read();
class Params {
  constructor(private readonly file: string = 'pp-default.json') {}
  read() { try { fs.readFileSync(fx(this.file)); } catch {} }
}
new Params().read();
new Params('pp-given.json').read();
class Priv {
  #file = 'pr-1.json';
  static patch(p: Priv) { p.#file = 'pr-2.json'; }
  read() { try { fs.readFileSync(fx(this.#file)); } catch {} }
}
const pr = new Priv();
Priv.patch(pr);
pr.read();
class Compound {
  private p = 'cp';
  grow() { this.p += '-more'; }
  read() { try { fs.readFileSync(fx(this.p)); } catch {} }
}
const cp = new Compound();
cp.grow();
cp.read();
abstract class Shape {
  protected readonly kind: string = 'shape';
  abstract area(): number;
  file() { try { fs.readFileSync(fx('kind-' + this.kind + '.json')); } catch {} }
}
class Circle extends Shape {
  protected readonly kind = 'circle';
  area() { return 1; }
}
new Circle().file();
class Lazy {
  private cache?: string;
  get file() { return (this.cache ??= 'lazy.json'); }
  read() { try { fs.readFileSync(fx(this.file)); } catch {} }
}
new Lazy().read();
class Holder {
  private readonly files: string[] = ['ho-1.json'];
  add(f: string) { this.files.push(f); }
  read() { for (const f of this.files) { try { fs.readFileSync(fx(f)); } catch {} } }
}
const ho = new Holder();
ho.add('ho-2.json');
ho.read();
class Loop {
  private idx = 0;
  private readonly names = ['lp-0.json', 'lp-1.json', 'lp-2.json'];
  read() { try { fs.readFileSync(fx(this.names[this.idx++])); } catch {} }
}
const lp = new Loop();
lp.read();
lp.read();
