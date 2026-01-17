import { execFileSync } from "child_process";
import path from "node:path";

// TODO: make this customizable?
const DIRECTORY = process.cwd();

const VENV_NAME = ".cram_env";

// disable package manager update notifiers
process.env.NO_UPDATE_NOTIFIER = 1;

const isWindows = process.platform === "win32";

// Make virtualenv
execFileSync("python3", ["-m", "venv", VENV_NAME]);

// Upgrade pip
execFileSync(getVenvBin("python3"), [
  "-m",
  "pip",
  "install",
  "--quiet",
  "--upgrade",
  "pip"
]);

// Install prysk
execFileSync(getVenvBin("pip"), ["install", "prysk==0.15.2"]);

// Which tests do we want to run?
const testArg = process.argv[3] ? process.argv[3] : process.argv[2];
const tests = isWindows ? testArg.replaceAll("/", path.sep) : testArg;

if (!tests) {
  throw new Error("No tests specified");
}

const args = [
  "--shell=bash",
  ...(process.env.PRYSK_INTERACTIVE === "true" ? ["--interactive"] : []),
  ...(isWindows ? ["--dos2unix"] : []),
  tests
];

const pryskExecutable = getVenvBin("prysk");
console.log(`Running ${pryskExecutable} ${args.join(" ")}`);

try {
  execFileSync(pryskExecutable, args, { stdio: "inherit", env: process.env });
} catch (e) {
  // Swallow the node error stack trace. stdio: inherit should
  // already have the test failures printed. We don't need the Node.js
  // execution to also print its stack trace from execSync.
  process.exit(1);
}

function getVenvBin(tool) {
  const allowedVenvTools = ["python3", "pip", "prysk"];
  if (!allowedVenvTools.includes(tool)) {
    throw new Error(`Tool not allowed: ${tool}`);
  }

  const suffix = isWindows ? ".exe" : "";

  const venvPath = path.join(DIRECTORY, VENV_NAME);
  const venvBin = isWindows
    ? path.join(venvPath, "Scripts")
    : path.join(venvPath, "bin");

  return path.join(venvBin, tool + suffix);
}
