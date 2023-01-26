function func() {
  if (false) {
    require("fail");
    import("fail");
  }
  if (true) {
    require("./ok");
  }
  if (true) {
    require("./ok");
  } else {
    require("fail");
    import("fail");
  }
  if (false) {
    require("fail");
    import("fail");
  } else {
    require("./ok");
  }
}

it("should not follow conditional references", () => {
  func();

  expect(func.toString()).not.toContain("import(");
});

it("should allow replacements in IIFEs", () => {
  (function func() {
    if (false) {
      require("fail");
      import("fail");
    }
  })();
});

it("should evaluate process.turbopack", () => {
  if (process.turbopack) {
  } else {
    require("fail");
    import("fail");
  }
});
