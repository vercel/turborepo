import { execSync } from "child_process";
import path from "node:path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = __filename.replace(/[^/\\]*$/, "");

const venvName = ".cram_env";
const venvPath = path.join(__dirname, venvName);

export const isWindows = process.platform === "win32";

const venvBin = isWindows
  ? path.join(venvPath, "Scripts")
  : path.join(venvPath, "bin");

const allowedTools = ["python3", "pip", "prysk"];

export function debugVenv() {
  console.log(`ls ${venvPath}`);
  execSync(`ls -la ${venvPath}`, { stdio: "inherit" });

  console.log(`ls ${venvBin}`);
  execSync(`ls -la ${venvBin}`, { stdio: "inherit" });
}

export function getVenvBin(tool) {
  if (!allowedTools.includes(tool)) {
    throw new Error(`Tool not allowed: ${tool}`);
  }

  const suffix = isWindows ? ".exe" : "";

  return path.join(venvBin, tool + suffix);
}

export function makeVenv() {
  execSync(`python3 -m venv ${venvName}`, { stdio: "inherit" });
}
