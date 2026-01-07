import { DynamicLink } from "fumadocs-core/dynamic-link";
import { Tab, Tabs } from "fumadocs-ui/components/tabs";
import { TypeTable } from "fumadocs-ui/components/type-table";
import defaultMdxComponents from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
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
import { File, Files, Folder } from "./files";
import { InVersion } from "./in-version";
import { LinkToDocumentation } from "./link-to-documentation";
import { Mermaid } from "./mermaid";
import { Step, Steps } from "./steps";
import { ThemeAwareImage } from "./theme-aware-image";
import { Video } from "./video";

export const getMDXComponents = (
  components?: MDXComponents
): MDXComponents => ({
  ...defaultMdxComponents,
  ...components,

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

  ExperimentalBadge
});
