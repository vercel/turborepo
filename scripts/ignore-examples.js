/*

This script is used to determine when examples should be built on Vercel.
We use a custom script for this situation instead of `npx turbo-ignore` because
the examples are not workspaces within this repository, and we want to rebuild them
only when:

1. The examples themselves have changed
2. The turbo version has changed

We recommend using `npx turbo-ignore` in your own projects.

*/

const childProcess = require("child_process");

// https://vercel.com/support/articles/how-do-i-use-the-ignored-build-step-field-on-vercel
const ABORT_BUILD_CODE = 0;
const CONTINUE_BUILD_CODE = 1;

const continueBuild = () => process.exit(CONTINUE_BUILD_CODE);
const abortBuild = () => process.exit(ABORT_BUILD_CODE);

const example = process.argv[2];

const ignoreCheck = () => {
  // no app name (directory) was passed in via args
  if (!example) {
    console.log(
      `\u226B Could not determine example to check - continuing build...`
    );
    continueBuild();
  }

  console.log(
    `\u226B Checking for changes to "examples/${example}" or "turbo" version...`
  );

  // get all file names changed in last commit
  const fileNameList = childProcess
    .execSync("git diff --name-only HEAD~1")
    .toString()
    .trim()
    .split("\n");

  // check if any files in the example have changed,
  const exampleChanged = fileNameList.some((file) =>
    file.startsWith(`examples/${example}`)
  );
  // or if a new version of turbo has been released.
  const turboVersionChanged = fileNameList.some(
    (file) => file === "version.txt"
  );

  if (exampleChanged) {
    console.log(
      `\u226B Detected changes to examples/${example} - continuing build...`
    );
    continueBuild();
  }

  if (turboVersionChanged) {
    console.log(`\u226B New version of "turbo" detected - continuing build...`);
    continueBuild();
  }

  console.log(`\u226B No relevant changes detected, skipping build.`);
  abortBuild();
};

ignoreCheck();
