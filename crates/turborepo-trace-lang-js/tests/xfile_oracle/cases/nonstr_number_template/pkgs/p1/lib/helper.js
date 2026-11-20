// A number from the caller in a template.
export function load(dir, i) {
  return require(`${dir}/n${i}`);
}
