import Link from "next/link";
import { Callout } from "@/components/geistdocs/callout";

export const CreateTurboCallout = () => (
  <Callout type="info">
    This guide assumes you&apos;re using{" "}
    <Link href="/docs/getting-started/installation">create-turbo</Link> or a
    repository with a similar structure.
  </Callout>
);
