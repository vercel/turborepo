import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function lazy(spec) {
  if (isString(spec)) return () => import(spec).catch(() => null);
  return () => null;
}
