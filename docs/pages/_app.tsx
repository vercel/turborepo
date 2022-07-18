import "../styles.css";
import "nextra-theme-docs/style.css";
import "../custom.css";

import { SSRProvider } from "@react-aria/ssr";
import type { AppProps } from "next/app";
import type { ReactNode } from "react";

type NextraAppProps = AppProps & {
  Component: AppProps["Component"] & {
    getLayout: (page: ReactNode) => ReactNode;
  };
};

// Shim requestIdleCallback in Safari
if (typeof window !== "undefined" && !("requestIdleCallback" in window)) {
  window.requestIdleCallback = (fn) => setTimeout(fn, 1);
  window.cancelIdleCallback = (e) => clearTimeout(e);
}

export default function Nextra({ Component, pageProps }: NextraAppProps) {
  const getLayout = Component.getLayout || ((page) => page);

  return getLayout(
    <>
      <SSRProvider>
        <Component {...pageProps} />
      </SSRProvider>
    </>
  );
}
