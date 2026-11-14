export class Loader {
  #dir;
  constructor(dir) {
    this.#dir = dir;
  }
  load(name) {
    return require(this.#dir + '/' + name);
  }
}
