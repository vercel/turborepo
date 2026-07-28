import { createMdxComponents } from "@vercel/geistdocs/mdx";
import { DynamicLink } from "fumadocs-core/dynamic-link";
import { Accordion, Accordions } from "fumadocs-ui/components/accordion";
import { Heading } from "fumadocs-ui/components/heading";
import { Step, Steps } from "fumadocs-ui/components/steps";
import type { MDXComponents } from "mdx/types";
import { Mermaid } from "@/components/diagram/diagram";
import type { SerializedBadge } from "@/lib/rehype-strip-heading-jsx";
import { cn } from "@/lib/utils";
import { ExamplesTable } from "./examples-table";
import { ExperimentalBadge } from "./experimental-badge";
import { File, Files, Folder } from "./files";
import { InVersion } from "./in-version";
import { LinkToDocumentation } from "./link-to-documentation";
import { PackageManagerTabs, PlatformTabs, Tab, Tabs } from "./tabs";
import { ThemeAwareImage } from "./theme-aware-image";

const BADGE_COMPONENTS: Record<
  string,
  React.ComponentType<{ children?: string }>
> = {
  ExperimentalBadge
};

function HeadingWithBadges({
  as,
  "data-heading-badges": badgeData,
  children,
  ...rest
}: React.ComponentProps<"h1"> & {
  as: "h1" | "h2" | "h3" | "h4" | "h5" | "h6";
  "data-heading-badges"?: string;
}) {
  let badges: SerializedBadge[] = [];
  if (badgeData) {
    try {
      badges = JSON.parse(badgeData);
    } catch {
      // Ignore malformed data
    }
  }

  return (
    <Heading as={as} {...rest}>
      {children}
      {badges.map((badge, i) => {
        const Component = BADGE_COMPONENTS[badge.component];
        if (!Component) return null;
        return <Component key={i}>{badge.text}</Component>;
      })}
    </Heading>
  );
}

interface GetMDXComponentsOptions {
  components?: MDXComponents;
  /** Use the old site's typography styling for H1 elements (centered, semibold) */
  isBlog?: boolean;
}

export const getMDXComponents = (
  options: GetMDXComponentsOptions = {}
): MDXComponents => {
  const { components, isBlog } = options;

  return createMdxComponents({
    ...components,
    h2: (props: React.ComponentProps<"h2">) => (
      <HeadingWithBadges as="h2" {...props} />
    ),
    h3: (props: React.ComponentProps<"h3">) => (
      <HeadingWithBadges as="h3" {...props} />
    ),
    h4: (props: React.ComponentProps<"h4">) => (
      <HeadingWithBadges as="h4" {...props} />
    ),
    h5: (props: React.ComponentProps<"h5">) => (
      <HeadingWithBadges as="h5" {...props} />
    ),
    h6: (props: React.ComponentProps<"h6">) => (
      <HeadingWithBadges as="h6" {...props} />
    ),
    ...(isBlog && {
      h1: (props: React.ComponentProps<"h1">) => {
        const { className, ...rest } = props;
        return (
          <HeadingWithBadges
            className={cn(
              "font-semibold text-center text-4xl tracking-wide!",
              className
            )}
            as="h1"
            {...rest}
          />
        );
      }
    }),
    a: ({ href, ...props }: React.ComponentProps<"a">) =>
      href?.startsWith("/") ? (
        <DynamicLink
          className="font-normal text-primary no-underline"
          href={`/[lang]${href}`}
          {...props}
        />
      ) : (
        <a
          href={href}
          {...props}
          className="font-normal text-primary no-underline"
        />
      ),
    Tabs,
    Tab,
    PackageManagerTabs,
    PlatformTabs,
    Mermaid,
    LinkToDocumentation,
    Files,
    File,
    Folder,
    Steps,
    Step,
    ExamplesTable,
    Accordion,
    Accordions,
    ThemeAwareImage,
    InVersion,
    ExperimentalBadge
  });
};
