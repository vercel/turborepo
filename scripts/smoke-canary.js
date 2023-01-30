#!/usr/bin/env node

const { execSync } = require("child_process");

function exec({ title, command, options, conditions }) {
  console.log();
  if (title) {
    console.log(`ℹ️ ${title}`);
  }
  console.log(`Running: "${command}"`);
  try {
    const result = execSync(command, options).toString().trim();
    if (process.env.GITHUB_ACTIONS === "true") {
      console.log(`::group::"${command}" output`);
      console.log(result);
      console.log(`::endgroup::`);
    } else {
      console.log(result);
    }

    if (conditions && conditions.length > 0) {
      conditions.forEach((condition) => {
        assertOutput({ output: result, command, ...condition });
      });
    } else {
      return result;
    }
  } catch (err) {
    console.error(err);
    console.error(err.stdout.toString());
    process.exit(1);
  }
}

function getGlobalBinaryPath({ packageManager }) {
  switch (packageManager) {
    case "yarn":
      return execSync(`yarn global bin`).toString().trim();
    case "npm":
      return execSync(`npm root --global`).toString().trim();
    case "pnpm":
      return execSync(`pnpm bin --global`).toString().trim();
  }
}

function assertOutput({ output, command, expected, condition }) {
  if (condition === "includes") {
    if (output.includes(expected)) {
      console.log(`✅ "${command}" output includes "${expected}"`);
    } else {
      console.error(`❌ "${command}" output does not include "${expected}"`);
      process.exit(1);
    }
  }

  if (condition === "notIncludes") {
    if (!output.includes(expected)) {
      console.log(`✅ "${command}" output does not include "${expected}"`);
    } else {
      console.error(`❌ "${command}" output does not include "${expected}"`);
      process.exit(1);
    }
  }

  if (condition === "startsWith") {
    if (output.startsWith(expected)) {
      console.log(`✅ "${command}" output starts with "${expected}"`);
    } else {
      console.error(`❌ "${command}" output does not start with "${expected}"`);
      process.exit(1);
    }
  }
}

function installExample({ version, packageManager }) {
  exec({
    command: `npx create-turbo@${version} --use-${packageManager} .`,
    conditions: [
      {
        expected: "Success! Your new Turborepo is ready.",
        condition: "includes",
      },
    ],
  });
}

function installGlobalTurbo({ version, packageManager }) {
  if (packageManager === "pnpm" || packageManager === "npm") {
    exec({
      title: "Install global turbo",
      command: `${packageManager} install turbo@${version} --global`,
    });
  } else {
    exec({
      title: "Install global turbo",
      command: `${packageManager} global add turbo@${version}`,
    });
  }
}

function uninstallLocalTurbo({ packageManager }) {
  if (packageManager === "pnpm" || packageManager === "npm") {
    exec({
      title: "Uninstall local turbo",
      command: `${packageManager} uninstall turbo`,
    });
  } else {
    exec({
      title: "Uninstall local turbo",
      command: `${packageManager} remove turbo -W`,
    });
  }
}

function getTurboBinary({ installType, packageManager }) {
  if (installType === "global") {
    return "turbo";
  } else {
    if (packageManager === "npm") {
      return "./node_modules/.bin/turbo";
    } else {
      return `${packageManager} turbo`;
    }
  }
}

function logTurboDetails({ installType, packageManager }) {
  const turboBinary = getTurboBinary({ installType, packageManager });
  exec({ command: `${turboBinary} --version` });
  exec({ command: `${turboBinary} bin` });
}

function verifyLocalBinary({ installType, packageManager }) {
  const turboBinary = getTurboBinary({ installType, packageManager });
  exec({
    title: "Verify binary is not installed globally",
    command: `${turboBinary} bin`,
    conditions: [
      {
        expected:
          packageManager === "npm" ? "/usr/local/lib/node_modules" : "global",
        condition: "notIncludes",
      },
    ],
  });
}

function verifyGlobalBinary({ installType, packageManager }) {
  const packageManagerGlobalBinPath = getGlobalBinaryPath({ packageManager });
  const turboBinary = getTurboBinary({ installType, packageManager });
  exec({
    title: "Verify binary is installed globally",
    command: `${turboBinary} bin`,
    conditions: [
      {
        expected: packageManagerGlobalBinPath,
        condition: "startsWith",
      },
    ],
  });
}

function verifyFirstBuild({ installType, packageManager }) {
  const turboBinary = getTurboBinary({ installType, packageManager });
  exec({
    title: "Verify first turbo build is successful and not cached",
    command: `${turboBinary} build`,
    conditions: [
      { expected: "2 successful, 2 total", condition: "includes" },
      { expected: "0 cached, 2 total", condition: "includes" },
      { expected: "FULL TURBO", condition: "notIncludes" },
    ],
  });
}

function verifySecondBuild({ installType, packageManager }) {
  const turboBinary = getTurboBinary({ installType, packageManager });
  exec({
    title: "Verify second turbo build is successful and cached",
    command: `${turboBinary} build`,
    conditions: [
      { expected: "2 successful, 2 total", condition: "includes" },
      { expected: "2 cached, 2 total", condition: "includes" },
      { expected: "FULL TURBO", condition: "includes" },
    ],
  });
}

function local({ local, packageManager }) {
  // setup example
  installExample({ version: local.version, packageManager });
  verifyLocalBinary({ installType: "local", packageManager });
  logTurboDetails({ installType: "local", packageManager });

  // verify build is correctly cached
  verifyFirstBuild({ installType: "local", packageManager });
  verifySecondBuild({ installType: "local", packageManager });
}

function global({ local, global, packageManager }) {
  // setup example
  installExample({ version: local.version, packageManager });
  installGlobalTurbo({ version: global.version, packageManager });
  logTurboDetails({ installType: "global", packageManager });

  verifyLocalBinary({ installType: "global", packageManager });
  uninstallLocalTurbo({ packageManager });
  logTurboDetails({ installType: "global", packageManager });
  verifyGlobalBinary({ installType: "global", packageManager });

  // verify build is correctly cached
  verifyFirstBuild({ installType: "global", packageManager });
  verifySecondBuild({ installType: "global", packageManager });
}

function both({ local, global, packageManager }) {
  // setup example
  installExample({ version: local.version, packageManager });
  installGlobalTurbo({ version: global.version, packageManager });
  logTurboDetails({ installType: "global", packageManager });
  verifyLocalBinary({ installType: "global", packageManager });

  // verify build is correctly cached
  verifyFirstBuild({ installType: "global", packageManager });
  verifySecondBuild({ installType: "global", packageManager });
}

const tests = {
  local,
  global,
  both,
};

function test() {
  const args = process.argv.slice(2);
  const [
    testName = "local",
    packageManager = "pnpm",
    localVersion = "canary",
    globalVersion = "canary",
  ] = args;

  const local = {
    type: "local",
    version: localVersion,
  };
  const global = {
    type: "global",
    version: globalVersion,
  };

  console.log(
    `Running test: "${testName}" with local version: "turbo@${localVersion}" and global version: turbo@${globalVersion} using ${packageManager}`
  );
  tests[testName]({ local, global, packageManager });
  console.log("Tests passed!");
}

test();
