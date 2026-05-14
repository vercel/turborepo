import assert from "node:assert/strict";
import test from "node:test";

import {
  createTurboRunTerminalOptions,
  sanitizeTurboRunTaskName
} from "./turbo-run-terminal-options";

const validTaskNames = [
  "build",
  "build:prod",
  "lint-staged",
  "test.unit",
  "web#build",
  "@acme/web#build",
  "//#build"
];

const invalidTaskNames = [
  "",
  "-build",
  " build",
  "build ",
  "foo bar",
  "foo; touch /tmp/pwned",
  "foo && touch /tmp/pwned",
  "foo | sh",
  "foo $(touch /tmp/pwned)",
  "foo`touch /tmp/pwned`",
  "foo\n touch /tmp/pwned",
  "foo' && touch /tmp/pwned && '",
  "foo & calc",
  "foo ^& calc"
];

for (const [index, taskName] of validTaskNames.entries()) {
  test(`passes valid task name ${index} as one terminal argument`, () => {
    assert.equal(sanitizeTurboRunTaskName(taskName), taskName);

    const options = createTurboRunTerminalOptions(
      "/Applications/Turbo CLI/bin/turbo",
      taskName
    );

    assert.equal(options.name, taskName);
    assert.equal(options.shellPath, "/Applications/Turbo CLI/bin/turbo");
    assert.deepEqual(options.shellArgs, ["run", taskName]);
  });
}

for (const [index, taskName] of invalidTaskNames.entries()) {
  test(`rejects invalid task name ${index}`, () => {
    assert.equal(sanitizeTurboRunTaskName(taskName), undefined);
  });
}

test("rejects non-string task names", () => {
  assert.equal(sanitizeTurboRunTaskName(undefined), undefined);
  assert.equal(sanitizeTurboRunTaskName(["build"]), undefined);
});
