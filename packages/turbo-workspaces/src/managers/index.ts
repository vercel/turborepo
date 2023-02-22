import pnpm from "./pnpm";
import npm from "./npm";
import yarn from "./yarn";
import { ManagerHandler, PackageManager } from "../types";

const MANAGERS: Record<PackageManager, ManagerHandler> = {
  pnpm,
  yarn,
  npm,
};
export default MANAGERS;
