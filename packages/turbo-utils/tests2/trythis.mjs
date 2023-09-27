import test from "node:test";
import { strict as assert } from "node:assert";
import { convertCase } from "../dist/convertCase.js";

test("synchronous passing test", () => {
  // This test passes because it does not throw an exception.
  assert.strictEqual(1, 1);
});

test("convertCase", () => {
  assert.strictEqual(convertCase("hello_world", { to: "camel" }), "helloWorld");
});

// describe("convertCase", () => {
//   const testCases: Array<TestCase> = [
//     { input: "hello_world", expected: "helloWorld", to: "camel" },
//     { input: "hello-world", expected: "helloWorld", to: "camel" },
//     { input: "helloWorld", expected: "helloWorld", to: "camel" },
//     { input: "helloworld", expected: "helloworld", to: "camel" },
//   ];

//   it.each(testCases)(
//     "should convert '$input' to '$to'",
//     ({ input, expected, to }) => {
//       expect(convertCase(input, { to })).toBe(expected);
//     }
//   );
// });
