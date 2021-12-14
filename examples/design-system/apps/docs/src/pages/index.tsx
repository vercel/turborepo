import { Button } from "@acme/button";
import { Card } from "@acme/card";
import { useIsomorphicLayoutEffect } from "@acme/utils";

export default function Docs() {
  useIsomorphicLayoutEffect(() => {
    console.log("Acme docs page");
  }, []);
  return (
    <div
      style={{ fontFamily: "Helvetica Neue", maxWidth: 800, margin: "0 auto" }}
    >
      <h1>acme Documentation</h1>
      <Card>
        <h3>Button</h3>
        <Button>Hello World</Button>
      </Card>
      <div style={{ height: 16 }}></div>
      <Card>
        <h3>Card</h3>
        <Card>Children</Card>
      </Card>
    </div>
  );
}
