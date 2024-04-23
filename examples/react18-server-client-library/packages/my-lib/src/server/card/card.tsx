import * as React from "react";

interface CardProps {
  className?: string;
  title: string;
  children: React.ReactNode;
  href: string;
}

/**
 * # Card
 * Card component to display cards with links.
 *
 * This component should be created as a server component as it does not have any client side functionality.
 * Therefore, we have not used "use client" here and also kept it under `server` directory.
 *
 * Looking for a much more practical example for use of client and
 * server components together in a library? Check out - https://github.com/react18-tools/nextjs-themes
 *
 * @param className - CSS className
 * @param title - Card title
 * @param children - Card description provided as children
 * @param href - Link to the resource
 */

export function Card({
  className,
  title,
  children,
  href,
}: CardProps): JSX.Element {
  return (
    <a
      className={className}
      href={href}
      rel="noopener noreferrer"
      target="_blank"
    >
      <h2>
        {title} <span>-&gt;</span>
      </h2>
      <p>{children}</p>
    </a>
  );
}
