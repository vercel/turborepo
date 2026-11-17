const { greet } = require("../src/greet");

test("greets", () => {
  expect(greet("trace")).toBe("Hello, trace!");
});
