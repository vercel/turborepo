import Link from "next/link";
import { Callout } from "#components/callout.tsx";

export function CreateTurboCallout(): JSX.Element {
  return (
    <Callout type="good-to-know">
      {" "}
      This guide assumes you&apos;re using{" "}
      <Link href="/docs/getting-started/installation">create-turbo</Link> or a
      repository with a similar structure.
    </Callout>
  );
}
