// `import()` in TypeScript type positions is type-only (M2).
type NodeFetch3RequestInit = NotAny<import('../node_modules/node-fetch').RequestInit> | NotAny<import('../../node_modules/node-fetch').RequestInit>;

type Mod = typeof import('./typeof-mod');
type Bare = import('./bare-type');
type Union =
  | import('./union-a').A
  | import('./union-b').B;

let annotated: import('./annotated').T;
function fn(param: import('./param').P): import('./ret').R {
  return null!;
}
interface Shape {
  field: import('./interface-field').F;
  method(): import('./interface-method').M;
}
declare const declared: import('./declared').D;
const generic: Map<string, import('./generic').G> = new Map();
class C {
  prop: import('./class-prop').P | undefined;
}

// Runtime imports stay runtime.
const lazy = import('./runtime-lazy');
const mod = await import('./runtime-await');
const obj = { a: import('./runtime-object') };
const pick = flag ? import('./runtime-then') : import('./runtime-else');
switch (k) {
  case 'x': import('./runtime-case'); break;
  default: import('./runtime-default');
}
import('./runtime-chain').then((m) => m.default);
const typed: Promise<unknown> = import('./runtime-typed-var');
