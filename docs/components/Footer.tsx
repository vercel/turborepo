import { useRouter } from "next/router";
import Link from "next/link";
import type { ReactNode, ReactElement } from "react";
import { useState } from "react";
import cn from "classnames";
import { ThemeSwitch } from "nextra-theme-docs";
import { Vercel } from "./logos/Vercel";
import type { TurboSite } from "./SiteSwitcher";
import { useTurboSite } from "./SiteSwitcher";

function FooterLink({ href, children }: { href: string; children: ReactNode }) {
  const classes =
    "text-sm text-[#666666] dark:text-[#888888] no-underline betterhover:hover:text-gray-700 betterhover:hover:dark:text-white transition";
  if (href.startsWith("http")) {
    return (
      <a className={classes} href={href}>
        {children}
      </a>
    );
  }
  return (
    <Link className={classes} href={href}>
      {children}
    </Link>
  );
}

function FooterHeader({ children }: { children: ReactNode }) {
  return <h3 className="text-sm text-black dark:text-white">{children}</h3>;
}

const navigation = {
  general: [
    { name: "Blog", href: "/blog" },
    { name: "Releases", href: "https://github.com/vercel/turbo/releases" },
    { name: "Governance", href: "/governance" },
  ],
  repo: [
    { name: "Documentation", href: "/repo/docs" },
    {
      name: "API Reference",
      href: "/repo/docs/reference/command-line-reference",
    },
    { name: "FAQ", href: "/repo/docs/faq" },
  ],
  pack: [
    { name: "Documentation", href: "/pack/docs" },
    { name: "Features", href: "/pack/docs/features" },
  ],
  support: [
    {
      name: "GitHub",
      href: "https://github.com/vercel/turbo",
    },
    {
      name: "Discord",
      href: "https://turbo.build/discord",
    },
  ],
  company: (site: TurboSite) => [
    { name: "Vercel", href: "https://vercel.com" },
    {
      name: "Open Source Software",
      href: "https://vercel.com/oss?utm_source=turbo.build&utm_medium=referral&utm_campaign=footer-ossLink",
    },
    {
      name: "Contact Sales",
      href: `https://vercel.com/${
        site === "repo" ? "solutions/turborepo" : "contact/sales"
      }?utm_source=turbo.build&utm_medium=referral&utm_campaign=footer-enterpriseLink`,
    },
    { name: "X", href: "https://x.com/vercel" },
  ],
  legal: [
    { name: "Privacy Policy", href: "/privacy" },
    { name: "Terms of Service", href: "/terms" },
  ],
};

export function FooterContent() {
  // Turborepo is used by default
  const site = useTurboSite() ?? "repo";
  return (
    <div aria-labelledby="footer-heading" className="w-full">
      <h2 className="sr-only" id="footer-heading">
        Footer
      </h2>
      <div className="w-full py-8 mx-auto">
        <div className="xl:grid xl:grid-cols-3 xl:gap-8">
          <div className="grid grid-cols-1 gap-8 xl:col-span-2">
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-6 md:gap-8">
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Resources</FooterHeader>
                <ul className="mt-4 space-y-1.5 list-none ml-0">
                  {navigation.general.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Turborepo</FooterHeader>
                <ul className="mt-4 space-y-1.5 list-none ml-0">
                  {navigation.repo.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Turbopack</FooterHeader>
                <ul className="mt-4 space-y-1.5 list-none ml-0">
                  {navigation.pack.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Company</FooterHeader>
                <ul className="mt-4 space-y-1.5 list-none ml-0">
                  {navigation.company(site).map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Legal</FooterHeader>
                <ul className="mt-4 space-y-1.5 list-none ml-0">
                  {navigation.legal.map((item) => (
                    <li key={item.name}>
                      <FooterLink href={item.href}>{item.name}</FooterLink>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="mt-12 md:!mt-0">
                <FooterHeader>Support</FooterHeader>
                <ul className="mt-4 space-y-1.5 list-none ml-0">
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
              Subscribe to the Turbo newsletter and stay updated on new releases
              and features, guides, and case studies.
            </p>
            <SubmitForm />
          </div>
        </div>

        <div className="pt-8 mt-8 sm:flex sm:items-center sm:justify-between">
          <div>
            <a
              className="text-current"
              href="https://vercel.com?utm_source=turbo.build&utm_medium=referral&utm_campaign=footer-logoLink"
              rel="noopener noreferrer"
              target="_blank"
              title="vercel.com homepage"
            >
              <Vercel />
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

function SubmitForm() {
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
            return router.push("/confirm");
          })
          .catch((e: unknown) => {
            if (e instanceof Error) {
              // eslint-disable-next-line no-console -- We'd like to see something weird is happening in Logs.
              console.error(e.message);
            }
            return router.push("/confirm");
          });
        ev.preventDefault();
      }}
    >
      <label className="sr-only" htmlFor="email-address">
        Email address
      </label>
      <input
        autoComplete="email"
        className="border-[#666666] dark:border-[#888888] w-full min-w-0 px-4 py-2 text-base text-gray-900 placeholder-gray-500 bg-white border rounded-md appearance-none dark:text-white sm:text-sm dark:bg-transparent focus:outline-none focus:ring-2 focus:ring-gray-800 dark:focus:border-white focus:placeholder-gray-400"
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
      <div className="mt-3 rounded-md sm:mt-0 sm:ml-3 sm:flex-shrink-0">
        <button
          className="flex items-center justify-center w-full px-4 py-2 text-base font-medium text-white bg-black border border-transparent rounded-md dark:bg-white dark:text-black sm:text-sm betterhover:hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-gray-800 dark:focus:ring-white dark:betterhover:hover:bg-gray-300"
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
    <footer className="bg-[#FAFAFA] pb-[env(safe-area-inset-bottom)] relative dark:bg-[#111111]">
      <div className="absolute top-0 h-12 w-full -translate-y-full bg-gradient-to-t from-[#FAFAFA] to-transparent dark:from-black pointer-events-none" />
      <div
        className={cn(
          "mx-auto max-w-[90rem] py-2 px-4 flex gap-2",
          menu ? "flex" : "hidden"
        )}
      >
        <ThemeSwitch />
      </div>
      <hr className="dark:border-neutral-800" />
      <div
        className={cn(
          "mx-auto max-w-[90rem] py-12 flex justify-center md:justify-center text-black dark:text-white",
          "pl-[max(env(safe-area-inset-left),1.5rem)] pr-[max(env(safe-area-inset-right),1.5rem)]"
        )}
      >
        <FooterContent />
      </div>
    </footer>
  );
}
