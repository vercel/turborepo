import * as React from "react";
import { Button } from "@repo/ui/button";

export default function IndexPage() {
  return (
    <main>
      <h1>Web</h1>
      <Button />
    </main>
  );
}

export function Head() {
  return <title>Home Page</title>;
}
