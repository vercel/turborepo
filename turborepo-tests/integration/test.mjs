import { execSync } from "child_process";
import path from "node:path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = __filename.replace(/[^/\\]*$/, "");

const venvName = ".cram_env";
const venvPath = path.join(__dirname, venvName);
const allowedVenvTools = ["python3", "pip", "prysk", "frysk"];

const isWindows = process.platform === "win32";

const venvBin = isWindows
  ? path.join(venvPath, "Scripts")
  : path.join(venvPath, "bin");

execSync(`python3 -m venv ${venvName}`, { stdio: "inherit" });

const python3 = getVenvBin("python3");
const pip = getVenvBin("pip");

console.log("install latest pip");
execSync(`${python3} -m pip install --quiet --upgrade pip`, {
  stdio: "inherit",
});

console.log("install prysk");
execSync(`${pip} install "frysk"`, { stdio: "inherit" });

// disable package manager update notifiers
process.env.NO_UPDATE_NOTIFIER = 1;

let specificTest = "tests";
let testArg = process.argv[2];
if (testArg) {
  if (isWindows) {
    testArg = testArg.replaceAll("/", path.sep);
  }
  specificTest = path.join("tests", testArg);
}

const pryskBin = getVenvBin("prysk");
const flags = [
  "--shell=bash",
  isWindows ? "--dos2unix" : "",
  process.env.PRYSK_INTERACTIVE === "true" ? "--interactive" : "",
].join(" ");

const cmd = `${pryskBin} ${flags} "${specificTest}"`;
console.log(`Running ${cmd}`);

try {
  execSync(cmd, { stdio: "inherit", env: process.env });
} catch (e) {
  // Swallow the node error stack trace. stdio: inherit should
  // already have the test failures printed. We don't need the Node.js
  // execution to also print its stack trace from execSync.
  process.exit(1);
}

function getVenvBin(tool) {
  if (!allowedVenvTools.includes(tool)) {
    throw new Error(`Tool not allowed: ${tool}`);
  }

  const suffix = isWindows ? ".exe" : "";

  return path.join(venvBin, tool + suffix);
}
