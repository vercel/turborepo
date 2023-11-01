import { execSync } from "child_process";
import { getVenvBin, makeVenv } from "./util.mjs";

makeVenv();

const python3 = getVenvBin("python3");
const pip = getVenvBin("pip");

console.log("install latest pip");
execSync(`${python3} -m pip install --quiet --upgrade pip`, {
  stdio: "inherit",
});

console.log("install prysk@15");

execSync(`${pip} install "prysk"`, { stdio: "inherit" });
