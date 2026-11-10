import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function otherVar(spec, o) {
  if (isString(o)) return import(spec).catch(() => null);
  return null;
}
