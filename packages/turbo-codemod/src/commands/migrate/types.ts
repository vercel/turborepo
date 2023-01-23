import { TransformerOptions } from "../../types";

export type MigrateCommandArgument = "string" | undefined;

export interface MigrateCommandOptions extends TransformerOptions {
  from?: string;
  to?: string;
  install: boolean;
}
