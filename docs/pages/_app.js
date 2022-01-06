import "../styles.css";
import "../nextra-theme-docs/styles.css";
import { SSRProvider } from "@react-aria/ssr";
import Prism from "prism-react-renderer/prism";

(typeof global !== "undefined" ? global : window).Prism = Prism;
require("prismjs/components/prism-docker");

// Some of the NPM packages we included, use following methods without polyfills.
// So, we need to include these polyfills.
// (This is only needed for Safari)
if (typeof window !== "undefined" && !("requestIdleCallback" in window)) {
  window.requestIdleCallback = (fn) => setTimeout(fn, 1);
  window.cancelIdleCallback = (e) => clearTimeout(e);
}

export default function Nextra({ Component, pageProps }) {
  return (
    <>
      <SSRProvider>
        <Component {...pageProps} />
      </SSRProvider>
    </>
  );
}
