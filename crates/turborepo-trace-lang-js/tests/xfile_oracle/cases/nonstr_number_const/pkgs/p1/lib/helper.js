// A constant of another file and a number from the caller.
import { BASE } from './config';
export function load(i) {
  return require(BASE + i);
}
