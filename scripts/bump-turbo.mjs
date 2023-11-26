import { exec } from "child_process";

const okMessage = /No staged files found/;

import { join } from "node:path";
import cp from "node:child_process";
import { promisify } from "node:util";
import { readdir, stat } from "node:fs/promises";
import { fileURLToPath } from "url";

const currentModuleURL = import.meta.url;
const currentModulePath = fileURLToPath(currentModuleURL);
const currentDirectory = path.dirname(currentModulePath);

function getPrettyName(example) {
  return `./examples/${example}`;
}

async function commitChanges(example) {
  const cmd2 = `git commit -am "chore(examples): bump turbo in examples/${example}"`;
  console.log(`⏵ running ${cmd2}`);

  return new Promise((resolve, reject) => {
    exec(cmd2, (err, stdout, stderr) => {
      if (!err) {
        console.log(`✔ changes committed`);
        resolve();
      } else {
        if (okMessage.test(err.message)) {
          // We don't care about this error message, we can just resolve.
          console.log(`✔ no changes to commit`);
          resolve();
        } else {
          if (process.env.DEBUG) {
            console.log(e.stdout);
            console.log(e.stderr);
          }

          console.log(`❌ failed to commit changes: ${err.message}}`);
          reject();
        }
      }
    });
  });
}

async function runCodemod(example) {
  const cmd1 = `npx @turbo/codemod update --force ./examples/${example}`;
  console.log(`⏵ Running ${cmd1}`);

  return new Promise((resolve, reject) => {
    exec(cmd1, (err, stdout, stderr) => {
      if (err) {
        if (process.env.DEBUG) {
          console.log(stdout);
          console.log(stderr);
        }
        console.log(`❌ @turbo/codemod failed: ${err.message}`);
        reject();
      } else {
        console.log(`✔ @turbo/codemod successful`);
        resolve();
      }
    });
  });
}

async function main() {
  const examplesDir = join(currentDirectory, "..", "examples");
  const examples = await readdir(examplesDir);

  // We don't try to use Promise.all here because @turbo/codemod does not like dirty
  // git state, so we have to run them one at a time.
  for (const example of examples) {
    const dir = join(examplesDir, example);
    const prettyName = getPrettyName(example);

    console.log(`↓ ${prettyName}`);

    if (!(await stat(dir)).isDirectory()) {
      console.log(`⚠️ skipping, not a directory`);
      console.log();
      continue;
    }

    try {
      await runCodemod(example);
    } catch (_) {
      // Continue for loop if something went wrong
      console.log();
      continue;
    }

    try {
      await commitChanges(example);
    } catch (_) {
      // if there was a problem with committing changes, we should break out of the loop.
      console.log();
      break;
    }

    console.log("✅ Done!");
    console.log();
  }
}

main();
