import { useRouter } from "next/router";

const ignoredRoutes = ["/blog", "/terms", "/privacy", "/confirm"];

const downrankedRoutes = [
  "/repo/docs/acknowledgements",
  // Deprecations
  "/repo/docs/core-concepts/scopes",
];

export function Main(props) {
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

  // Hiding the footer from search.
  return (
    <main data-pagefind-body {...props}>
      {props.children.props.children.slice(0, -1)}
      <div data-pagefind-ignore="all">
        {
          props.children.props.children[
            props.children.props.children.length - 1
          ]
        }
      </div>
    </main>
  );
}
