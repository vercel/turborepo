import Link from "next/link";

type LinkProps = Parameters<typeof Link>[0];

/** Link to either external or internal documentation. */
export const LinkToDocumentation = (props: LinkProps) => {
  return (
    <small>
      <Link {...props}>→ {props.children}</Link>
    </small>
  );
};
