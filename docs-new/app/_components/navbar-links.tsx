"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const baseLinks = [
  { href: "/blog", label: "Blog" },
  { href: "/showcase", label: "Showcase" },
  { href: "/enterprise", label: "Enterprise" },
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
    <>
      {links.map((link) => (
        <Link
          key={link.href}
          href={link.href}
          className="text-[#eaeaea] hover:opacity-75 ml-4"
        >
          {link.label}
        </Link>
      ))}
    </>
  );
};
