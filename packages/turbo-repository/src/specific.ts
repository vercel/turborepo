// Concrete type for JS ecosystem.
// Pros: Most freedom, allows for closer modeling of ecosystem-specific behavior
// Cons: requires a whole set of types per ecosystem
class JSRepository {
  static fromPath(path: string): JSRepository {
    throw new Error("not yet implemented");
  }
}
