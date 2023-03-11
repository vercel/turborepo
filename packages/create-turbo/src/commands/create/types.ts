export type CreateCommandArgument = "string" | undefined;

export interface CreateCommandOptions {
  skipInstall?: boolean;
  example?: string;
  examplePath?: string;
}
