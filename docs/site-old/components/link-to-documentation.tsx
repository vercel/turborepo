import Link from "next/link";
import type { LinkProps as NextLinkProps } from "next/link";
import type { ReactNode } from "react";

type LinkProps = NextLinkProps & {
  children: ReactNode;
};

/** Link to either external or internal documentation. */
export function LinkToDocumentation(props: LinkProps): JSX.Element {
  return (
    <small>
      <Link className="flex flex-row gap-2 space-y-0" {...props}>
        <span>→</span> {props.children}
      </Link>
    </small>
  );
}
