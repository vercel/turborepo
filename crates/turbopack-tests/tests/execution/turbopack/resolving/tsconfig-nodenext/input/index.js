import foo from "./src/foo.js";

it("should correctly resolve explicit extensions with nodenext", () => {
  expect(foo).toBe("foo");
});
