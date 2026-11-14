// Called as a template tag: the first argument is the array of the template's strings.
export function load(strings) {
  return require(strings[0]);
}
