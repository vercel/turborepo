import { execSync } from "child_process";

const venvName = ".cram_env";
const venvPath = path.join(__dirname, venvName);
const venvBin = path.join(venvPath, "bin");
const venvPython = path.join(venvBin, "python3");
const venvPip = path.join(venvBin, "pip");

execSync("python3 -m venv .cram_env");
execSync(`${venvPython} -m pip install --quiet --upgrade pip`);
execSync(`${venvPip} install "prysk==0.15.0"`);
