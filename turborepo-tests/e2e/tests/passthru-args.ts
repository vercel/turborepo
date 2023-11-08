import * as assert from "uvu/assert";
import * as uvu from "uvu";
import { getCommandOutputAsArray } from "../helpers";
import { PackageManager } from "../types";
import { Monorepo } from "../monorepo";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options: { cwd?: string } = {}
) {
  return suite(`${pkgManager} passes through correct args`, async () => {
    const expectArgsPassed = (inputArgs: string[], passedArgs: string[]) => {
      const result = getCommandOutputAsArray(
        repo.turbo("run", inputArgs, options)
      );
      // Find the output logs of the test script
      const needle = "//:args: Output:";
      const script_output = result.find((line) => line.startsWith(needle));

      assert.ok(
        script_output != undefined && script_output.startsWith(needle),
        `Unable to find '//:arg' output in '${result}'`
      );
      const [node, ...args] = JSON.parse(
        script_output.substring(needle.length)
      );

      assert.match(
        node,
        "node",
        `Expected node binary path (${node}) to contain 'node'`
      );
      assert.equal(args, passedArgs);
    };

    const tests = [
      [["args", "--filter=//", "--", "--script-arg=42"], ["--script-arg=42"]],
      [["args", "--filter=//", "--", "--filter=//"], ["--filter=//"]],
      [["--filter=//", "args", "--", "--filter=//"], ["--filter=//"]],
      [
        ["args", "--", "--script-arg", "42"],
        ["--script-arg", "42"],
      ],
      [["args"], []],
      [["args", "--"], []],
      [
        ["args", "--", "--", "--"],
        ["--", "--"],
      ],
      [
        ["args", "--", "first", "--", "second"],
        ["first", "--", "second"],
      ],
      [
        ["args", "--", "-f", "--f", "---f", "----f"],
        ["-f", "--f", "---f", "----f"],
      ],
    ];

    for (const [input, expected] of tests) {
      expectArgsPassed(input, expected);
    }
  });
}
