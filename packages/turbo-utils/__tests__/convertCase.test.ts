import { convertCase, type CaseOptions } from "../src/convertCase";

interface TestCase {
  input: string;
  expected: string;
  to: CaseOptions["to"];
}

describe("convertCase", () => {
  const testCases: Array<TestCase> = [
    { input: "hello_world", expected: "helloWorld", to: "camel" },
    { input: "hello-world", expected: "helloWorld", to: "camel" },
    { input: "helloWorld", expected: "helloWorld", to: "camel" },
    { input: "helloworld", expected: "helloworld", to: "camel" },
  ];

  it.each(testCases)(
    "should convert '$input' to '$to'",
    ({ input, expected, to }) => {
      expect(convertCase(input, { to })).toBe(expected);
    }
  );
});
