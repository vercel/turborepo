it("should load chunk a", () => {
  expect(import("./a")).resolves.toHaveProperty("default", "a");
});

it("should load chunk b", () => {
  expect(import("./b")).resolves.toHaveProperty("default", "b");
});
