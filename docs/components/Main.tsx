import type { ReactNode } from "react";
import { useRouter } from "next/router";
import { ignoredRoutes, downrankedRoutes } from "../lib/search";

export function Main(props: { children: ReactNode }) {
  const router = useRouter();

  if (ignoredRoutes.some((route) => route === router.asPath)) {
    return <main {...props} />;
  }

  if (
    router.asPath.startsWith("/blog") ||
    downrankedRoutes.some((route) => route === router.asPath)
  ) {
    return (
      <main data-pagefind-body data-pagefind-weight=".2">
        {props.children}
      </main>
    );
  }

  interface NestedProps {
    props: { children: ReactNode[] };
  }

  const footerNode = (props.children as NestedProps).props.children;

  // Hiding the "previous page, next page" footer from search
  // because it produces many erroneous results.
  // We don't need to worry about adding this ignore to the returns above
  // because those entire pages are already either ignored or downranked.
  return (
    <main data-pagefind-body {...props}>
      {footerNode.slice(0, -1)}
      <div data-pagefind-ignore="all">{footerNode.at(-1)}</div>
    </main>
  );
}
