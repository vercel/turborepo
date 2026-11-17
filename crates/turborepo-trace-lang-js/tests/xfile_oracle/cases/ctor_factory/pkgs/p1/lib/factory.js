// The instance is made in another file; the test calls its method.
import { Loader } from './loader';
export function make(dir) {
  return new Loader(dir);
}
