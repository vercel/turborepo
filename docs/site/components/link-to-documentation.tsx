import Link from "next/link";

type LinkProps = Parameters<typeof Link>[0];

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
