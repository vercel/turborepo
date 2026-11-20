import { DynamicLink } from "fumadocs-core/dynamic-link";

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
    {href.startsWith("/") ? (
      <DynamicLink
        className="inline-flex flex-row gap-2 space-y-0 decoration-foreground decoration-1"
        href={`/[lang]${href}`}
        prefetch
      >
        → {text}
      </DynamicLink>
    ) : (
      <a
        className="inline-flex flex-row gap-2 space-y-0 decoration-foreground decoration-1"
        href={href}
      >
        → {text}
      </a>
    )}
  </small>
);
