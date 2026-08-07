import type { PackageManager } from "../types";
import type { ManagerHandler } from "../types";
import { aube } from "./aube";
import { nub } from "./nub";
import { utoo } from "./utoo";
import { pnpm } from "./pnpm";
import { npm } from "./npm";
import { yarn } from "./yarn";
import { bun } from "./bun";

export const MANAGERS: Record<PackageManager, ManagerHandler> = {
  aube,
  nub,
  utoo,
  pnpm,
  yarn,
  npm,
  bun
};
