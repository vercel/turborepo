import { transform as packageManagerTransform } from "./package-manager";
import { transform as officialStarter } from "./official-starter";
import { transform as gitIgnoreTransform } from "./git-ignore";
import { transform as pnpmEslintTransform } from "./pnpm-eslint";
import type { TransformInput, TransformResult } from "./types";

/**
 * In the future, we may want to support sourcing additional transforms from the templates themselves.
 */
export const transforms: Array<(args: TransformInput) => TransformResult> = [
  officialStarter,
  gitIgnoreTransform,
  packageManagerTransform,
  pnpmEslintTransform,
];
