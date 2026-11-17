import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function wrongArg(spec, w) {
  if (other(spec, w)) return import(spec).catch(() => null);
  return null;
}
