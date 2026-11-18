import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function otherwise(spec) {
  if (isString(spec)) return null;
  else return import(spec).catch(() => null);
}
