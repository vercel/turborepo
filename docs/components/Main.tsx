import type { ReactNode } from "react";
import { useRouter } from "next/router";
import { weightedRoutes } from "../lib/search";

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
    <div {...rest}>
      {contentNodes.slice(0, -1)}
      <div data-pagefind-ignore="all">{contentNodes.at(-1)}</div>
    </div>
  );
}

export function Main(props: { children: ReactNode }) {
  const router = useRouter();

  if (router.asPath.startsWith("/blog")) {
    return <Layout data-pagefind-body data-pagefind-weight=".2" {...props} />;
  }

  if (weightedRoutes.some((route) => route[0] === router.asPath)) {
    return (
      <Layout
        data-pagefind-body
        data-pagefind-weight={
          weightedRoutes.find((route) => route[0] === router.asPath)?.[1] ?? 1
        }
        {...props}
      />
    );
  }

  return <Layout data-pagefind-body {...props} />;
}
