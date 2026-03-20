import { DynamicLink } from "fumadocs-core/dynamic-link";
import { Heading } from "fumadocs-ui/components/heading";
import { TypeTable } from "fumadocs-ui/components/type-table";
import { PackageManagerTabs, PlatformTabs, Tab, Tabs } from "./tabs";
import defaultMdxComponents from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
import type { SerializedBadge } from "@/lib/rehype-strip-heading-jsx";
import { cn } from "@/lib/utils";
import { Accordion, Accordions } from "./accordion";
import {
  Callout,
  CalloutContainer,
  CalloutDescription,
  CalloutTitle
} from "./callout";
import { Card, Cards } from "./card";
import { CodeBlock } from "./code-block";
import {
  CodeBlockTab,
  CodeBlockTabs,
  CodeBlockTabsList,
  CodeBlockTabsTrigger
} from "./code-block-tabs";
import { ExamplesTable } from "./examples-table";
import { ExperimentalBadge } from "./experimental-badge";
import { PrereleaseBadge } from "./prerelease-badge";
import { File, Files, Folder } from "./files";
import { InVersion } from "./in-version";
import { LinkToDocumentation } from "./link-to-documentation";
import { Mermaid } from "@/components/diagram/diagram";
import { Step, Steps } from "./steps";
import { ThemeAwareImage } from "./theme-aware-image";
import { Video } from "./video";

const BADGE_COMPONENTS: Record<
  string,
  React.ComponentType<{ children?: string }>
> = {
  ExperimentalBadge,
  PrereleaseBadge
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

  return {
    ...defaultMdxComponents,
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
    pre: CodeBlock,
    a: ({ href, ...props }) =>
      href.startsWith("/") ? (
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
    CodeBlockTabs,
    CodeBlockTabsList,
    CodeBlockTabsTrigger,
    CodeBlockTab,
    TypeTable,
    Tabs,
    Tab,
    PackageManagerTabs,
    PlatformTabs,
    Callout,
    CalloutContainer,
    CalloutTitle,
    CalloutDescription,
    Mermaid,
    Video,
    LinkToDocumentation,
    Cards,
    Card,
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
    ExperimentalBadge,
    PrereleaseBadge
  };
};
