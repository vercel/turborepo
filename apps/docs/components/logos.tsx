import React, { type JSX } from "react";
import { cn } from "@/lib/utils";

export function VercelLogo({ className }: { className?: string }): JSX.Element {
  return (
    <svg
      className={cn(className, "fill-black dark:fill-white")}
      fill="none"
      height={22}
      viewBox="0 0 235 203"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M117.082 0L234.164 202.794H0L117.082 0Z" fill="currentColor" />
    </svg>
  );
}

export function TurborepoLogo({
  className,
  monochrome = false
}: {
  className?: string;
  monochrome?: boolean;
}): JSX.Element {
  const gradientId = `turborepo-logo-gradient-${React.useId().replace(/:/g, "")}`;

  return (
    <svg
      className={className}
      aria-label="Turborepo logomark"
      height="80"
      role="img"
      viewBox="0 0 77 77"
      width="80"
    >
      <defs>
        <linearGradient
          id={gradientId}
          x1="41.443"
          y1="11.5372"
          x2="10.5567"
          y2="42.4236"
          gradientUnits="userSpaceOnUse"
        >
          <stop stopColor="#0096FF" />
          <stop offset="1" stopColor="#FF1E56" />
        </linearGradient>
      </defs>
      <path
        className="fill-black dark:fill-white"
        d="M38.5017 18.0956C27.2499 18.0956 18.0957 27.2498 18.0957 38.5016C18.0957 49.7534 27.2499 58.9076 38.5017 58.9076C49.7535 58.9076 58.9077 49.7534 58.9077 38.5016C58.9077 27.2498 49.7535 18.0956 38.5017 18.0956ZM38.5017 49.0618C32.6687 49.0618 27.9415 44.3346 27.9415 38.5016C27.9415 32.6686 32.6687 27.9414 38.5017 27.9414C44.3347 27.9414 49.0619 32.6686 49.0619 38.5016C49.0619 44.3346 44.3347 49.0618 38.5017 49.0618Z"
      />
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M40.2115 14.744V7.125C56.7719 8.0104 69.9275 21.7208 69.9275 38.5016C69.9275 55.2824 56.7719 68.989 40.2115 69.8782V62.2592C52.5539 61.3776 62.3275 51.0644 62.3275 38.5016C62.3275 25.9388 52.5539 15.6256 40.2115 14.744ZM20.5048 54.0815C17.233 50.3043 15.124 45.4935 14.7478 40.2115H7.125C7.5202 47.6025 10.4766 54.3095 15.1088 59.4737L20.501 54.0815H20.5048ZM36.7916 69.8782V62.2592C31.5058 61.883 26.695 59.7778 22.9178 56.5022L17.5256 61.8944C22.6936 66.5304 29.4006 69.483 36.7878 69.8782H36.7916Z"
        fill={monochrome ? "currentColor" : `url(#${gradientId})`}
      />
    </svg>
  );
}

export function GithubLogo({ className }: { className?: string }): JSX.Element {
  return (
    <svg
      className={className}
      height="24"
      shapeRendering="geometricPrecision"
      viewBox="0 0 24 24"
      width="24"
    >
      <path
        d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"
        fill="currentColor"
      />
    </svg>
  );
}
