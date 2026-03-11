import { DynamicLink } from "fumadocs-core/dynamic-link";
import { Heading } from "fumadocs-ui/components/heading";
import { TypeTable } from "fumadocs-ui/components/type-table";
import { PackageManagerTabs, PlatformTabs, Tab, Tabs } from "./tabs";
import defaultMdxComponents from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
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
    ...(isBlog && {
      h1: (props: React.ComponentProps<"h1">) => {
        const { className, ...rest } = props;
        return (
          <Heading
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
