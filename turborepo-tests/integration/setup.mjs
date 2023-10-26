import { execSync } from "child_process";
import { debugVenv, getVenvBin, makeVenv } from "./util.mjs";

makeVenv();

execSync(`echo "pwd: $(pwd)"`, { stdio: "inherit" });

debugVenv();

const python3 = getVenvBin("python3");
const pip = getVenvBin("pip");

execSync(`${python3} -m pip install --quiet --upgrade pip`);
execSync(`${pip} install "prysk==0.15.0"`);
