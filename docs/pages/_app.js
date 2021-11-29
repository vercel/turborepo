import "../styles.css";
import "../nextra-theme-docs/styles.css";

export default function Nextra({ Component, pageProps }) {
  return (
    <>
      <Component {...pageProps} />
    </>
  );
}
