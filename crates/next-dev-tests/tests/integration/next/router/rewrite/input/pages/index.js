import { useEffect } from "react";

export default function Foo() {
  useEffect(() => {
    // Only run on client
    import("@turbo/pack-test-harness").then(runTests);
  });

  return "index";
}

function runTests() {
  it("it should display foo, not index", () => {
    throw new Error("index.js page loaded, it should be rewritten to foo.js");
  });
}
