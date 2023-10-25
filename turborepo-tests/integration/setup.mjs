import { execSync } from "child_process";

execSync("python3 -m venv .cram_env");
execSync(".cram_env/bin/python3 -m pip install --quiet --upgrade pip");
execSync('.cram_env/bin/pip install "prysk==0.15.0"');
