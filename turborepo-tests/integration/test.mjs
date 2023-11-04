import { execSync } from "child_process";
import path from "node:path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = __filename.replace(/[^/\\]*$/, "");

const venvName = ".cram_env";
const isWindows = process.platform === "win32";

// Make virtualenv
execSync(`python3 -m venv ${venvName}`);

// Get executables
const python3 = getVenvBin("python3");
const pip = getVenvBin("pip");

// Install pip and prysk
console.log("install latest pip");
execSync(`${python3} -m pip install --quiet --upgrade pip`, {
  stdio: "inherit",
});

console.log("install prysk");
execSync(`${pip} install "frysk"`, { stdio: "inherit" }); // TODO: move this back to prysk once https://github.com/prysk/prysk/pull/207 is merged

// disable package manager update notifiers
process.env.NO_UPDATE_NOTIFIER = 1;

// Which tests do we want to run?
let testArg = process.argv[2] ? process.argv[2] : "";
testArg = isWindows ? testArg.replaceAll("/", path.sep) : testArg;

// What flags to pass to prysk?
const flags = [
  "--shell=bash",
  isWindows ? "--dos2unix" : "",
  process.env.PRYSK_INTERACTIVE === "true" ? "--interactive" : "",
].join(" ");

const cmd = [
  getVenvBin("prysk"), // prysk program
  flags, // flags for the program
  path.join("tests", testArg), // arguments for the program
].join(" ");

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
  const allowedVenvTools = ["python3", "pip", "prysk", "frysk"];

  if (!allowedVenvTools.includes(tool)) {
    throw new Error(`Tool not allowed: ${tool}`);
  }

  const suffix = isWindows ? ".exe" : "";

  const venvPath = path.join(__dirname, venvName);
  const venvBin = isWindows
    ? path.join(venvPath, "Scripts")
    : path.join(venvPath, "bin");

  return path.join(venvBin, tool + suffix);
}
