import * as React from "react";
export const NewTabLink = ({
  children,
  href,
  ...other
}: {
  children: React.ReactNode;
  href: string;
}) => {
  return (
    <a target="_blank" rel="noreferrer" href={href} {...other}>
      {children}
    </a>
  );
};
