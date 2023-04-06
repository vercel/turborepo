export type CreateCommandArgument = "string" | undefined;

export interface CreateCommandOptions {
  skipInstall?: boolean;
  skipTransforms?: boolean;
  example?: string;
  examplePath?: string;
}
