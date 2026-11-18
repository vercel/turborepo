import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function reassigned(spec, alt) {
  if (isString(spec)) {
    spec = alt;
    return import(spec).catch(() => null);
  }
  return null;
}
