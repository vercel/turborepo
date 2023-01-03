import { useEffect } from "react";
import { Inter } from "@next/font/google";
import { Deferred } from "@turbo/pack-test-harness/deferred";

const interNoArgs = Inter();

let testResult = new Deferred();

export default function Home() {
  useEffect(() => {
    // Only run on client
    import("@turbo/pack-test-harness").then(runTests);
  });

  return <div className={interNoArgs.className}>Test</div>;
}

globalThis.waitForTests = function () {
  return testResult.promise;
};

function runTests() {
  it("returns structured data about the font styles from the font function", () => {
    expect(interNoArgs).toEqual({
      className:
        "className◽[project-with-next]/crates/next-dev/tests/integration/next/font-google/basic/[embedded_modules]/@vercel/turbopack-next/internal/font/google/inter_34ab8b4d.module.css",
      style: {
        fontFamily: "'__Inter_34ab8b4d'",
        fontStyle: "normal",
      },
    });
  });

  it("loads the font face for Inter matching ascii ranges", async () => {
    expect(
      [...(await document.fonts.ready)].filter((f) => f.status === "loaded")
    ).toMatchObject([
      {
        ascentOverride: "normal",
        descentOverride: "normal",
        display: "optional",
        family: "__Inter_34ab8b4d",
        featureSettings: "normal",
        lineGapOverride: "normal",
        sizeAdjust: "100%",
        status: "loaded",
        stretch: "normal",
        style: "normal",
        unicodeRange:
          "U+0-FF, U+131, U+152-153, U+2BB-2BC, U+2C6, U+2DA, U+2DC, U+2000-206F, U+2074, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD",
        variant: "normal",
        weight: "100 900",
      },
    ]);
  });

  testResult.resolve(__jest__.run());
}
