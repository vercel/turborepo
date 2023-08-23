import * as React from "react";

export function Card({
  title,
  cta,
  href,
}: {
  title: string;
  cta: string;
  href: string;
}): JSX.Element {
  return (
    <a
      className="ui-group ui-mt-4 ui-rounded-lg ui-border ui-border-transparent ui-overflow-hidden ui-bg-origin-border ui-bg-gradient-to-r ui-from-brandred ui-to-brandblue ui-text-[#6b7280]"
      href={href}
      rel="noopener noreferrer"
      target="_blank"
    >
      <div className="ui-p-4 ui-bg-zinc-900 ui-h-full">
        <p className="ui-inline-block ui-text-xl ui-text-white">{title}</p>
        <div className="ui-text-xs ui-mt-4 group-hover:ui-underline">
          {cta} →
        </div>
      </div>
    </a>
  );
}
