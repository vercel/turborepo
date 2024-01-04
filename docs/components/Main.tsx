import type { ReactNode } from "react";
import { useRouter } from "next/router";
import { ignoredRoutes, downrankedRoutes, uprankedRoutes } from "../lib/search";

interface NestedProps {
  props: { children: ReactNode[] };
}

// Hiding the "previous page, next page" footer from search
// because it produces erroneous results.
// We don't need to worry about adding this ignore to the returns above
// because those entire pages are already either ignored or downranked.
function Layout({ children, ...rest }: { children: ReactNode }) {
  const contentNodes = (children as NestedProps).props.children;

  return (
    <main {...rest}>
      {contentNodes.slice(1, -1)}
      <div data-pagefind-ignore="all">{contentNodes.at(-1)}</div>
    </main>
  );
}

export function Main(props: { children: ReactNode }) {
  const router = useRouter();

  if (ignoredRoutes.some((route) => route === router.asPath)) {
    return <Layout data-pagefind-ignore="all" {...props} />;
  }

  if (
    downrankedRoutes.some((route) => route === router.asPath) ||
    router.asPath.startsWith("/blog")
  ) {
    return <Layout data-pagefind-body data-pagefind-weight=".2" {...props} />;
  }

  if (uprankedRoutes.some((route) => route === router.asPath)) {
    return <Layout data-pagefind-body data-pagefind-weight="1.2" {...props} />;
  }

  return <Layout data-pagefind-body {...props} />;
}
