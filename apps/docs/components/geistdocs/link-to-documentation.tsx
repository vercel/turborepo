import Link from "next/link";

type LinkToDocumentationProps = {
  href: string;
  text: string;
};

/** Link to either external or internal documentation. */
export const LinkToDocumentation = ({
  href,
  text
}: LinkToDocumentationProps) => (
  <small className="not-prose underline">
    <Link
      href={href}
      className="inline-flex flex-row gap-2 space-y-0 decoration-foreground decoration-1"
    >
      → {text}
    </Link>
  </small>
);
