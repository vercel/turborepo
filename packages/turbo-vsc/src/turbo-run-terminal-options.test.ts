import assert from "node:assert/strict";
import test from "node:test";

import { createTurboRunTerminalOptions } from "./turbo-run-terminal-options";

const taskNames = [
  "build",
  "build:prod",
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

for (const [index, taskName] of taskNames.entries()) {
  test(`passes task name ${index} as one terminal argument`, () => {
    const options = createTurboRunTerminalOptions(
      "/Applications/Turbo CLI/bin/turbo",
      taskName
    );

    assert.equal(options.name, taskName);
    assert.equal(options.shellPath, "/Applications/Turbo CLI/bin/turbo");
    assert.deepEqual(options.shellArgs, ["run", taskName]);
  });
}
