it("polyfills `global` to `globalThis`", () => {
  expect(global).toEqual(globalThis);
});
