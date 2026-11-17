import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function both(spec, on) {
  if (on && isString(spec)) return import(spec).catch(() => null);
  return null;
}
