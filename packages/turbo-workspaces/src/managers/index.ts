import type { PackageManager } from "../types";
import type { ManagerHandler } from "../types";
import { nub } from "./nub";
import { pnpm } from "./pnpm";
import { npm } from "./npm";
import { yarn } from "./yarn";
import { bun } from "./bun";

export const MANAGERS: Record<PackageManager, ManagerHandler> = {
  nub,
  pnpm,
  yarn,
  npm,
  bun
};
