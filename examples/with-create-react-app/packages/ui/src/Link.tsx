import * as React from "react";

interface LinkProps extends React.AnchorHTMLAttributes<HTMLAnchorElement> {
  children: React.ReactNode;
  href: string;
}

export function Link(props: LinkProps): JSX.Element {
  const { children, href, ...rest } = props;

  if (rest.target === "_blank") {
    rest.rel = "noopener noreferrer";
  }

  return (
    <a href={href} {...rest}>
      {children}
    </a>
  );
}
