"use client";

import classNames from "classnames";
import Link from "next/link";
import { usePathname } from "next/navigation";

const baseLinks = [
  { href: "/blog", label: "Blog" },
  { href: "/showcase", label: "Showcase" },
  {
    href: "https://vercel.com/contact/sales?utm_source=turbo.build&utm_medium=referral&utm_campaign=header-enterpriseLink",
    label: "Enterprise",
  },
];

export const NavbarLinks = () => {
  const pathname = usePathname();

  let links = [];

  if (pathname === "/repo" || pathname === "/pack") {
    links = [{ href: `${pathname}/docs`, label: "Docs" }, ...baseLinks];
  } else {
    links = baseLinks;
  }

  return (
    <div className="flex w-full gap-4 justify-end">
      {links.map((link) => (
        <Link
          key={link.href}
          href={link.href}
          target={link.href.startsWith("http") ? "_blank" : undefined}
          className={classNames(
            pathname.startsWith(link.href)
              ? "font-medium text-sm"
              : "text-gray-400 text-sm hover:text-white"
          )}
        >
          {link.label}
        </Link>
      ))}
    </div>
  );
};
