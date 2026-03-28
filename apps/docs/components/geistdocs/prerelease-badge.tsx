import { Badge } from "./badge";

/**
 * A custom variant of the Badge for prerelease features
 *
 * NOTE: children are supported but constrained to strings to support utilizing this
 * component in MDX linkable headings.
 */
export function PrereleaseBadge({
  children
}: {
  isLink?: boolean;
  children?: string;
}) {
  return <Badge className="text-white">{children || "Prerelease"}</Badge>;
}
