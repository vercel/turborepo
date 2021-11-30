import "../styles.css";
import "../nextra-theme-docs/styles.css";
import { SSRProvider } from "@react-aria/ssr";
export default function Nextra({ Component, pageProps }) {
  return (
    <>
      <SSRProvider>
        <Component {...pageProps} />
      </SSRProvider>
    </>
  );
}
