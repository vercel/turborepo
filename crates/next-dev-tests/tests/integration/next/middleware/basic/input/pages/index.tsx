import { useEffect } from "react";

export default function Home() {
  useEffect(() => {
    // Only run on client
    import("@turbo/pack-test-harness").then(runTests);
  });

  return null;
}

function runTests() {
  it("should allow redirects to other paths", async () => {
    const res = await fetch("/about/hello");
    expect(res.url.endsWith("/about-2"));
  });
}
