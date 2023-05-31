export type CreateCommandArgument = string | undefined;

export interface CreateCommandOptions {
  skipInstall?: boolean;
  skipTransforms?: boolean;
  turboVersion?: string;
  example?: string;
  examplePath?: string;
}
