import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function load(spec) {
  if (Array.isArray(spec)) return Promise.all(spec.map((s) => load(s)));
  if (isString(spec)) return import(spec).catch(() => null);
  return load(spec.handler);
}
