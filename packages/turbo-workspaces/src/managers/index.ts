import type { ManagerHandler, PackageManager } from "../types";
import { pnpm } from "./pnpm";
import { npm } from "./npm";
import { yarn } from "./yarn";

export const MANAGERS: Record<PackageManager, ManagerHandler> = {
  pnpm,
  yarn,
  npm,
};
