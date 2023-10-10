import { fail } from "assert";
import fs from "fs/promises";
import path from "path";
console.log(path.join(process.cwd(), "./test-results/main"));

const resultsDir = path.join(process.cwd(), "./test-results/main");
// get a list of all the file names in the test-results/main directory
//const files = await fs.readdir(resultsDir);
const files = ["202310090855-v13.5.5-canary.4-7e7a5fc.json"];

let passingTests = "";
let failingTests = "";

try {
  // loop over the files, read the contents and parse the JSON
  for (const file of files) {
    let passCount = 0;
    let failCount = 0;
    // skip if file name does not start with a number
    //if (!file.match(/^\d+/)) continue;
    let timestamp = `${file.slice(0, 4)}-${file.slice(4, 6)}-${file.slice(
      6,
      8
    )} ${file.slice(8, 10)}:${file.slice(10, 12)}:00`;

    const contents = await fs.readFile(path.join(resultsDir, file), "utf-8");
    const results = JSON.parse(contents);
    let { ref } = results;

    for (const result of results.result) {
      let suitePassCount = 0;
      let suiteFailCount = 0;

      suitePassCount += result.data.numPassedTests;
      suiteFailCount += result.data.numFailedTests;

      let suiteName = result.data.testResults[0].name;
      // remove "/home/runner/work/turbo/turbo/" from the beginning of suiteName
      suiteName = suiteName.slice(30);
      if (suitePassCount > 0) {
        passingTests += `${suiteName}\n`;
      }

      if (suiteFailCount > 0) {
        failingTests += `${suiteName}\n`;
      }

      for (const assertionResult of result.data.testResults[0]
        .assertionResults) {
        let assertion = assertionResult.fullName.replaceAll("`", "\\`");
        if (assertionResult.status === "passed") {
          passingTests += `* ${assertion}\n`;
        } else if (assertionResult.status === "failed") {
          failingTests += `* ${assertion}\n`;
        }
      }

      passCount += suitePassCount;
      failCount += suiteFailCount;

      if (suitePassCount > 0) {
        passingTests += `\n`;
      }

      if (suiteFailCount > 0) {
        failingTests += `\n`;
      }
    }

    console.log(`${ref} ${timestamp} ${passCount}/${passCount + failCount}`);

    // do something with the results
  }

  // write passing tests to file
  // await fs.writeFile(
  //   path.join(process.cwd(), "./passing-tests.txt"),
  //   passingTests
  // );

  // await fs.writeFile(
  //   path.join(process.cwd(), "./failing-tests.txt"),
  //   failingTests
  // );
} catch (error) {
  console.log(error);
}
