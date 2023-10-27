// disable package manager update notifiers
import { execSync } from "child_process";
import { getVenvBin, isWindows } from "./util.mjs";

process.env.NO_UPDATE_NOTIFIER = 1;
const specificTest = process.argv[2];

const pryskBin = getVenvBin("prysk");

console.log(`Running ${specificTest || "all"} tests... with ${pryskBin}`);

const testArg = specificTest ? `tests/${specificTest}` : "tests";

try {
  execSync(`${pryskBin} --shell="bash" "${testArg}"`, { stdio: "inherit" });
} catch (e) {
  // Swallow the node error stack trace. stdio: inherit should
  // already have the test failures printed. We don't need the Node.js
  // execution to also print its stack trace from execSync.
  process.exit(1);
}
