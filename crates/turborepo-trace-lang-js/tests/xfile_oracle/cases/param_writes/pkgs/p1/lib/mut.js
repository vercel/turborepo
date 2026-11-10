// The names are built at run time: no string of this file names a module.
const n = (c) => './mods/' + String.fromCharCode(c);
// Each function changes the object it gets in one way; `second` leaves its first
// parameter alone, `first` its second.
let kept;
const api = {
  set(z) {
    z.name = n(112);
  },
};
function inner(z) {
  z.name = n(101);
}
function innerBound(z) {
  z.name = n(114);
}
function ident(q) {
  return q;
}
function innerWrapped(z) {
  z.name = n(111);
}
export function second(a, b) {
  b.name = n(98);
}
export function first(a, b) {
  a.name = n(116);
}
export function alias(a) {
  const t = a;
  t.name = n(99);
}
export function viaArgs() {
  arguments[0].name = n(100);
}
export function viaLocal(a) {
  inner(a);
}
export function later(a) {
  return () => {
    a.name = n(102);
  };
}
export function stash(a) {
  kept = a;
}
export function flush() {
  kept.name = n(103);
}
export function deep(w) {
  w.inner.name = n(104);
}
export function viaRest(...xs) {
  xs[1].name = n(105);
}
export function destr({ o }) {
  o.name = n(106);
}
export function swap(a, b) {
  let t = b;
  t = a;
  t.name = n(107);
}
export function loop(xs) {
  for (const x of xs) x.name = n(108);
}
export function assign(a) {
  Object.assign(a, { name: n(109) });
}
export function wrapcall(a) {
  innerWrapped(ident(a));
}
export function method(a) {
  api.set(a);
}
export function cond(a, b, f) {
  (f ? a : b).name = n(113);
}
export function bound(a) {
  innerBound.call(null, a);
}
export function cb(a) {
  [a].forEach((z) => {
    z.name = n(115);
  });
}
export function computed(a, key) {
  a[key] = n(117);
}
export function viaDefault(a, b = a) {
  b.name = n(118);
}
export function shadow(a) {
  {
    const a = { name: 'x' };
    a.name = n(120);
  }
}
export function nestedParam(a) {
  const run = (a) => {
    a.name = n(119);
  };
  run({ name: 'x' });
}
