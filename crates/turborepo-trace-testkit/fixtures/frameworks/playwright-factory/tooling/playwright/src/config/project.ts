// A wrapper that is not the identity: it adds a field. A runner that follows factories sees
// `passWithNoTests`; one that does not treats `defineProject(x)` of a package as `x`.
export function defineProject<T extends object>(config: T): T & { passWithNoTests: boolean } {
  return { ...config, passWithNoTests: true };
}
