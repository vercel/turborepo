"use client";

import { useRouter } from "next/navigation";
import Link from "next/link";
import type { ReactNode, ReactElement } from "react";
import { useState } from "react";
import { cn } from "#components/cn.ts";
import { gitHubRepoUrl } from "#lib/constants.ts";
import { VercelLogo } from "./logos";

function findError<T extends object>(error: T): boolean {
  if (Object.prototype.toString.call(error) === "[object Error]") {
    return true;
  }

  const prototype = Object.getPrototypeOf(error) as T | null;

  return prototype === null ? false : findError(prototype);
}

const isObject = (object: unknown): object is Record<string, unknown> => {
  return (
    typeof object === "object" && object !== null && !Array.isArray(object)
  );
};

const isError = (error: unknown): error is Error => {
  if (!isObject(error)) return false;

  // Check for `Error` objects instantiated within the current global context.
  if (error instanceof Error) return true;

  return findError(error);
};

function FooterLink({
  href,
  children,
  id,
}: {
  id?: string;
  href: string;
  children: ReactNode;
}): JSX.Element {
  const classes =
    "text-sm text-[#666666] dark:text-[#888888] no-underline betterhover:hover:text-gray-700 betterhover:hover:dark:text-white transition";
  if (href.startsWith("http")) {
    return (
      <a className={classes} href={href} id={id}>
        {children}
      </a>
    );
  }
  return (
    <Link className={classes} href={href} id={id}>
      {children}
    </Link>
  );
}

function FooterHeader({ children }: { children: ReactNode }): JSX.Element {
  return <h3 className="text-sm text-black dark:text-white">{children}</h3>;
}

const navigation = {
  general: [
    { name: "Blog", href: "/blog" },
    { name: "Releases", href: `${gitHubRepoUrl}/releases` },
    { name: "Governance", href: "/governance" },
  ],
  repo: [
    { name: "Documentation", href: "/docs" },
    {
      name: "API Reference",
      href: "/docs/reference",
    },
    { name: "Telemetry", href: "/docs/telemetry" },
  ],
  support: [
    {
      name: "GitHub",
      href: "https://github.com/vercel/turborepo",
    },
    {
      name: "Community",
      href: "https://community.vercel.com/tag/turborepo",
    },
  ],
  company: [
    { name: "Vercel", href: "https://vercel.com" },
    {
      name: "Open Source Software",
      href: "https://vercel.com/oss?utm_source=turborepo.com&utm_medium=referral&utm_campaign=footer-ossLink",
    },
    {
      name: "Contact Sales",
      href: "https://vercel.com/solutions/turborepo?utm_source=turborepo.com&utm_medium=referral&utm_campaign=footer-enterpriseLink",
    },
    { name: "X", href: "https://x.com/vercel" },
  ],
  legal: [
    { name: "Privacy Policy", href: "/privacy" },
    { name: "Terms of Service", href: "/terms" },
    { name: "Cookie Preferences", id: "fides-modal-link", href: "#" },
  ],
};

function FooterContent(): JSX.Element {
  return (
    <div aria-labelledby="footer-heading" className="w-full">
      <h2 className="sr-only" id="footer-heading">
        Footer
      </h2>
      <div className="mx-auto w-full py-8">
        <div className="xl:grid xl:grid-cols-3 xl:gap-8">
          <div className="grid grid-cols-1 gap-8 xl:col-span-2">
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-6 md:gap-8">
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Resources</FooterHeader>
                <ul className="ml-0 mt-4 list-none space-y-1.5">
                  {navigation.general.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Turborepo</FooterHeader>
                <ul className="ml-0 mt-4 list-none space-y-1.5">
                  {navigation.repo.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Company</FooterHeader>
                <ul className="ml-0 mt-4 list-none space-y-1.5">
                  {navigation.company.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Legal</FooterHeader>
                <ul className="ml-0 mt-4 list-none space-y-1.5">
                  {navigation.legal.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href} id={item.id || undefined}>
                        {item.name}
                      </FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Support</FooterHeader>
                <ul className="ml-0 mt-4 list-none space-y-1.5">
                  {navigation.support.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
            </div>
          </div>
          <div className="mt-12 xl:!mt-0">
            <FooterHeader>Subscribe to our newsletter</FooterHeader>
            <p className="mt-4 text-sm text-gray-600 dark:text-[#888888]">
              Subscribe to the Turborepo newsletter and stay updated on new
              releases and features, guides, and case studies.
            </p>
            <SubmitForm />
          </div>
        </div>

        <div className="mt-8 pt-8 sm:flex sm:items-center sm:justify-between">
          <div>
            <a
              className="text-current"
              href="https://vercel.com?utm_source=turborepo.com&utm_medium=referral&utm_campaign=footer-logoLink"
              rel="noopener noreferrer"
              target="_blank"
              title="vercel.com homepage"
            >
              <VercelLogo />
            </a>
            <p className="mt-4 text-xs text-gray-500 dark:text-[#888888]">
              &copy; {new Date().getFullYear()} Vercel, Inc. All rights
              reserved.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}

function SubmitForm(): JSX.Element {
  const [email, setEmail] = useState("");
  const router = useRouter();
  return (
    <form
      className="mt-4 sm:flex sm:max-w-md"
      onSubmit={(ev) => {
        fetch("/api/signup", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({ email }),
        })
          .then((res) => res.json())
          .then(() => {
            router.push("/confirm");
          })
          .catch((e: unknown) => {
            if (isError(e)) {
              // eslint-disable-next-line no-console -- We'd like to see something weird is happening in Logs.
              console.error(e.message);
            }
            router.push("/confirm");
          });
        ev.preventDefault();
      }}
    >
      <label className="sr-only" htmlFor="email-address">
        Email address
      </label>
      <input
        autoComplete="email"
        className="w-full min-w-0 appearance-none rounded-md border border-[#666666] bg-white px-4 py-2 text-base text-gray-900 placeholder-gray-500 focus:placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-gray-800 dark:border-[#888888] dark:bg-transparent dark:text-white dark:focus:border-white sm:text-sm"
        id="email-address"
        name="email-address"
        onChange={(e) => {
          setEmail(e.target.value);
        }}
        placeholder="you@example.com"
        required
        type="email"
        value={email}
      />
      <div className="mt-3 rounded-md sm:ml-3 sm:mt-0 sm:flex-shrink-0">
        <button
          className="betterhover:hover:bg-gray-600 dark:betterhover:hover:bg-gray-300 flex w-full items-center justify-center rounded-md border border-transparent bg-black px-4 py-2 text-base font-medium text-white focus:outline-none focus:ring-2 focus:ring-gray-800 dark:bg-white dark:text-black dark:focus:ring-white sm:text-sm"
          type="submit"
        >
          Subscribe
        </button>
      </div>
    </form>
  );
}

export function Footer({ menu }: { menu?: boolean }): ReactElement {
  return (
    <footer className="relative bg-background-100 pb-[env(safe-area-inset-bottom)] dark:bg-[#111111]">
      <div className="pointer-events-none absolute top-0 h-12 w-full -translate-y-full bg-gradient-to-t from-[#FAFAFA] to-transparent dark:from-black" />
      <div
        className={cn(
          "mx-auto flex max-w-[90rem] gap-2 px-4 py-2",
          menu ? "flex" : "hidden"
        )}
      />
      <hr className="dark:border-neutral-800" />
      <div
        className={cn(
          "mx-auto flex max-w-[90rem] justify-center py-12 text-black dark:text-white md:justify-center",
          "pl-[max(env(safe-area-inset-left),1.5rem)] pr-[max(env(safe-area-inset-right),1.5rem)]"
        )}
      >
        <FooterContent />
      </div>
    </footer>
  );
}
