import { execSync } from "child_process";

export function checkIfYarnInstalled(): boolean {
  try {
    execSync("yarnpkg --version", { stdio: "ignore" });
    return true;
  } catch (e) {
    return false;
  }
}
