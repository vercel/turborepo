import { useEffect } from "react";
import "./style.css";

const Home = () => {
  useEffect(() => {
    // Only run on client
    import("@turbo/pack-test-harness").then(runTests);
  });
  return <h1>pxtorem</h1>;
};

export default Home;

function runTests() {
  it("it should apply pxtorem", function () {
    const layer = document.styleSheets[0].cssRules[0];
    const h1 = layer.cssRules[0];
    const fontSize = h1.style.getPropertyValue("font-size");
    expect(fontSize).toBe("10rem");
  });
}
