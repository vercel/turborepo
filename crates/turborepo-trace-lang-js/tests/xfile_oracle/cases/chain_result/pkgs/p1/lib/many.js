export function many(specs) {
  return specs.map((s) => require(s));
}
