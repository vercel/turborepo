import { describe, test } from "node:test";
import { strict as assert } from "node:assert";
import { convertCase } from "../dist/convertCase.js";

const testCases = [
  { input: "hello_world", expected: "helloWorld", to: "camel" },
  { input: "hello-world", expected: "helloWorld", to: "camel" },
  { input: "helloWorld", expected: "helloWorld", to: "camel" },
  { input: "helloworld", expected: "helloworld", to: "camel" },
];

describe("convertCase", () => {
  for (const testCase of testCases) {
    test(`should convert ${testCase.input} to ${testCase.to}`, () => {
      const output = convertCase(testCase.input, { to: testCase.to });
      assert.strictEqual(output, testCase.expected);
    });
  }
});
