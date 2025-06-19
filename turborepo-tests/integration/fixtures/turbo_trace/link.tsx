export interface LinkProps {
  children: React.ReactNode;
  href: string;
}

export const Link = ({ children, href }: LinkProps) => {
  return <a href={href}>{children}</a>;
};
