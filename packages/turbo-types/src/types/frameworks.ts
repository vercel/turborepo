export type FrameworkStrategy = "all" | "some";

export interface EnvConditional {
  when: { key: string; value?: string; absent?: boolean };
  include: Array<string>;
  fromVersion?: string;
  untilVersion?: string;
}

export interface Framework {
  slug: string;
  name: string;
  envWildcards: Array<string>;
  envConditionals?: Array<EnvConditional>;
  dependencyMatch: {
    strategy: FrameworkStrategy;
    dependencies: Array<string>;
  };
}
