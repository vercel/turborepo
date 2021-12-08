import "../styles.css";
import "../nextra-theme-docs/styles.css";
import { SSRProvider } from "@react-aria/ssr";
import Prism from "prism-react-renderer/prism";

(typeof global !== "undefined" ? global : window).Prism = Prism;
require("prismjs/components/prism-docker");

export default function Nextra({ Component, pageProps }) {
  return (
    <>
      <SSRProvider>
        <Component {...pageProps} />
      </SSRProvider>
    </>
  );
}
