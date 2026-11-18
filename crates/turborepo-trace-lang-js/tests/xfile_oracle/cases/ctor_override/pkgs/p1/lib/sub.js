// A subclass in another file whose method changes the name before the load.
import { Loader } from './loader';
export class Sub extends Loader {
  constructor() {
    super('./other');
  }
  load(name) {
    return super.load(name.replace('-x', ''));
  }
}
