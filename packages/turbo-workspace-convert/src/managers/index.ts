import pnpm from "./pnpm";
import npm from "./npm";
import yarn from "./yarn";
import { ManagerHandler, PackageManagers } from "../types";

const MANAGERS: Record<PackageManagers, ManagerHandler> = {
  pnpm,
  npm,
  yarn,
};
export default MANAGERS;
