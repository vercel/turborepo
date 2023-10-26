import { execSync } from "child_process";
import { getVenvBin, makeVenv } from "./util.mjs";

makeVenv();

execSync(`${getVenvBin("python3")} -m pip install --quiet --upgrade pip`);
execSync(`${getVenvBin("pip")} install "prysk==0.15.0"`);
