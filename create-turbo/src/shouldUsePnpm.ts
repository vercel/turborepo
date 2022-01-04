import { execSync } from "child_process";

export function shouldUsePnpm(): boolean {
  try {
    const userAgent = process.env.npm_config_user_agent;
    if (userAgent && userAgent.startsWith("pnpm")) {
      return true;
    }
    execSync("pnpm --version", { stdio: "ignore" });
    return true;
  } catch (e) {
    return false;
  }
}

export function getNpxCommandOfPnpm() {
  const currentVersion = execSync("pnpm --version").toString();
  return Number(currentVersion.split(".")[0]) >= 6 ? "pnpm dlx" : "pnpx";
}
