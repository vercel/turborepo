import Link from "next/link";
import { Badge } from "./Badge";

/**
 * A custom variant of the Badge for experimental features
 *
 * NOTE: children are supported but constrained to strings to support utilizing this
 * component in MDX linkable headings.
 */
export function ExperimentalBadge({
  isLink = true,
  children,
}: {
  isLink?: boolean;
  children?: string;
}) {
  const badge = <Badge>{children || "Experimental"}</Badge>;
  if (isLink) {
    return (
      <Link href="/repo/docs/faq#what-does-experimental-mean">{badge}</Link>
    );
  }
  return badge;
}
