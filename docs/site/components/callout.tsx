import { Children, type ReactNode } from "react";
import { Callout as FumaCallout } from "fumadocs-ui/components/callout";
import { cn } from "./cn";

type CalloutType = "info" | "error" | "warn" | "good-to-know";

export type FumaCalloutProps = Omit<
  React.ComponentProps<typeof FumaCallout>,
  "type"
> & {
  type: CalloutType;
};

const THEMES: Record<
  CalloutType,
  {
    background: string;
    text: string;
    border: string;
  }
> = {
  info: {
    background: "bg-blue-200 dark:bg-blue-200 dark:bg-opacity-20",
    text: "text-blue-900 dark:text-blue-100/100",
    border: "border border-blue-400/40",
  },
  error: {
    background: "bg-red-200 dark:bg-red-200 dark:bg-opacity-20",
    text: "text-red-900 dark:text-red-100/100",
    border: "border border-red-400/40",
  },
  warn: {
    background: "bg-amber-200 dark:bg-amber-200 dark:bg-opacity-20",
    text: "text-amber-900 dark:text-amber-100/100",
    border: "border border-amber-400/40",
  },
  "good-to-know": {
    background: "bg-transparent",
    text: "text-gray-900",
    border: "border border-foreground/40",
  },
};

const iconStyles = "w-5 h-5 relative top-0.5";

// Typing the icons explicitly
const ICONS: Record<CalloutType, React.ReactNode> = {
  info: (
    <svg
      className={`${THEMES.info.text} ${iconStyles}`}
      height="14"
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
      width="14"
      aria-hidden="true"
      role="img"
    >
      <title>Information icon</title>
      <circle cx="12" cy="12" fill="transparent" r="10" />
      <path d="M12 16v-4" />
      <path d="M12 8h.01" />
    </svg>
  ),
  error: (
    <svg
      className={`${THEMES.error.text} ${iconStyles}`}
      fill="none"
      height="24"
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
      width="24"
      aria-hidden="true"
      role="img"
    >
      <title>Error icon</title>
      <path
        d="M7.86 2h8.28L22 7.86v8.28L16.14 22H7.86L2 16.14V7.86L7.86 2z"
        fill="transparent"
      />
      <path d="M12 8v4" />
      <path d="M12 16h.01" />
    </svg>
  ),
  warn: (
    <svg
      className={`${THEMES.warn.text} ${iconStyles}`}
      fill="none"
      height="24"
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
      width="24"
      aria-hidden="true"
      role="img"
    >
      <title>Warning icon</title>
      <path
        d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
        fill="transparent"
      />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </svg>
  ),
  "good-to-know": null,
};

export function Callout({
  type = "info",
  ...props
}: FumaCalloutProps): JSX.Element {
  // Ensure type is a valid CalloutType
  const safeType: CalloutType = type in THEMES ? (type as CalloutType) : "info";

  // Cast children to ReactNode[] for type safety
  const childrenToArray = Children.toArray(
    props.children as ReactNode | Array<ReactNode>
  ).filter(Boolean) as Array<ReactNode>;
  const goodToKnowChildren = [
    <p className="inline font-medium" key="good-to-know">
      Good to know:&nbsp;
    </p>,
    ...childrenToArray,
  ];

  // Safely access theme properties
  const theme = THEMES[safeType];
  const themeClasses = `${theme.background} ${theme.border} leading-6`;
  const icon = ICONS[safeType];

  return (
    <FumaCallout
      className={themeClasses}
      icon={icon}
      type={safeType as "info" | "error" | "warn"}
      {...props}
    >
      {safeType === "good-to-know" ? (
        <div className="[&>p:nth-child(2)]:ps-1 [&>p]:inline">
          {goodToKnowChildren}
        </div>
      ) : (
        <div className={cn("[&>p:nth-child(2)]:ps-1 [&>p]:inline", theme.text)}>
          {childrenToArray}
        </div>
      )}
    </FumaCallout>
  );
}
