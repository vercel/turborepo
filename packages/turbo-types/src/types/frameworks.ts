export type FrameworkStrategy = "all" | "some";

export interface Framework {
  slug: string;
  name: string;
  envWildcards: Array<string>;
  dependencyMatch: {
    strategy: FrameworkStrategy;
    dependencies: Array<string>;
  };
}
