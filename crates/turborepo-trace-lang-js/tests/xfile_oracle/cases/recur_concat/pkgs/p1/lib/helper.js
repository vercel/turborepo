// A recursive walk; the handler's extension is added at the load.
export function normalize(x) {
  const out = [];
  if (typeof x.handler === 'string') out.push(require(x.handler + '.js'));
  for (const c of x.children || []) out.push(...normalize(c));
  return out;
}
