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
    <aside className="flex flex-col gap-10 border-r border-border px-5 py-7 max-[720px]:gap-5 max-[720px]:border-r-0 max-[720px]:border-b max-[720px]:p-4">
      <Link
        className="flex items-center gap-2.5 font-semibold tracking-tight"
        href="/"
        aria-label="Turborepo Factory home"
      >
        <span
          className="h-0 w-0 border-r-8 border-b-[14px] border-l-8 border-r-transparent border-b-current border-l-transparent"
          aria-hidden="true"
        />
        <span>Turborepo Factory</span>
      </Link>
      <nav aria-label="Factory pages">
        <ul className="grid list-none gap-1 p-0 max-[720px]:flex max-[720px]:overflow-x-auto">
          {links.map(({ href, label }) => (
            <li key={href}>
              <Link
                className="block rounded-md px-2.5 py-2 text-sm text-muted-foreground no-underline hover:bg-accent hover:text-foreground data-[active=true]:bg-accent data-[active=true]:text-foreground max-[720px]:whitespace-nowrap"
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
