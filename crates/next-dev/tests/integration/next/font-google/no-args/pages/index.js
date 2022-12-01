import { useEffect } from "react";
import { Inter } from "@next/font/google";
import { Deferred } from "@turbo/pack-test-harness/deferred";

const inter = Inter();

let testResult = new Deferred();

export default function Home() {
  useEffect(() => {
    // Only run on client
    import("@turbo/pack-test-harness").then(runTests);
  });

  return (
    <div id="text" className={inter.className}>
      Text
    </div>
  );
}

globalThis.waitForTests = function () {
  return testResult.promise;
};

function runTests() {
  it("uses the requested font when className is used", function () {
    const text = document.getElementById("text");
    expect(getComputedStyle(text).fontFamily).toEqual("Inter");
  });

  it("loads fonts", function () {
    expect([...document.fonts].length).toEqual(7);
    for (font of document.fonts) {
      expect(font.family).toEqual("Inter");
    }
  });

  testResult.resolve(__jest__.run());
}
