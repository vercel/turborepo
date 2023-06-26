import * as React from "react";
import type { HeadFC, PageProps } from "gatsby";
import { Button } from "ui";

const IndexPage: React.FC<PageProps> = () => {
  return (
    <main>
      <h1>Web</h1>
      <Button />
    </main>
  );
};

export default IndexPage;

export const Head: HeadFC = () => <title>Home Page</title>;
