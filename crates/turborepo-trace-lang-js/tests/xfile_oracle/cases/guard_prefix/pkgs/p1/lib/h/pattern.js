// An anchored regular expression.
export function pattern(spec) {
  if (!/^\.\.\/area\/(c|d)\.js$/.test(spec)) throw new Error('outside');
  return import(spec).catch(() => null);
}
