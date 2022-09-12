import { execSync } from "child_process";
import { CommandName } from "./constants";

export const getPackageManagerVersion = (command: CommandName): string => {
  return execSync(`${command} --version`).toString().trim();
};
