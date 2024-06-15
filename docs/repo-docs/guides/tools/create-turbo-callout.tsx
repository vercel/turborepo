import Link from "next/link";
import { Callout } from "#/components/callout";

export function CreateTurboCallout(): JSX.Element {
  return (
    <Callout type="good-to-know">
      {" "}
      This guide assumes you&apos;re using{" "}
      <Link href="/repo/docs/getting-started/installation">
        create-turbo
      </Link>{" "}
      or a repository with a similar structure.
    </Callout>
  );
}
