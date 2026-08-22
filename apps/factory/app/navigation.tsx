"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const links = [
  { href: "/", label: "Agent Runs" },
  { href: "/work", label: "Start work" },
  { href: "/factory-image", label: "Factory image" }
];

export function Navigation() {
  const pathname = usePathname();
  return (
    <aside className="sidebar">
      <Link className="brand" href="/" aria-label="Turborepo Factory home">
        <span className="brandMark" aria-hidden="true" />
        <span>Turborepo Factory</span>
      </Link>
      <nav aria-label="Factory pages">
        <ul className="navigationList">
          {links.map(({ href, label }) => (
            <li key={href}>
              <Link
                className="navigationLink"
                data-active={pathname === href || undefined}
                href={href}
              >
                {label}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
    </aside>
  );
}
