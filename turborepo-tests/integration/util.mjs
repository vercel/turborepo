import { execSync } from "child_process";
import path from "node:path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = __filename.replace(/[^/\\]*$/, "");

const venvName = ".cram_env";
const venvPath = path.join(__dirname, venvName);
const venvBin = path.join(venvPath, "bin");

const allowedTools = ["python3", "pip", "prysk"];

export function debugVenv() {
  console.log(`venvPath: ${venvPath}`);
  consoe.log("venvPath contents");
  execSync(`ls -la ${venvPath}`, { stdio: "inherit" });
  consoe.log("venvBin contents");
  execSync(`ls -la ${venvBin}`, { stdio: "inherit" });
}

export function getVenvBin(tool) {
  if (!allowedTools.includes(tool)) {
    throw new Error(`Tool not allowed: ${tool}`);
  }

  return path.join(venvBin, tool);
}

export function makeVenv() {
  execSync(`python3 -m venv ${venvName}`, { stdio: "inherit" });
}
