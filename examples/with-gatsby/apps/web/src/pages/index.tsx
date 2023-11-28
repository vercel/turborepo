import * as React from "react";
import type { HeadProps, PageProps } from "gatsby";
import { Button } from "@repo/ui/button";

function IndexPage(_: PageProps): JSX.Element {
  return (
    <main>
      <h1>Web</h1>
      <Button />
    </main>
  );
}

export default IndexPage;

export function Head(_: HeadProps): JSX.Element {
  return <title>Home Page</title>;
}
