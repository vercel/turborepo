import type { MDXComponents } from "mdx/types";
import defaultComponents from "fumadocs-ui/mdx";
import { Pre, CodeBlock } from "fumadocs-ui/components/codeblock";
import { Heading } from "fumadocs-ui/components/heading";
import type { ReactNode } from "react";
import { NodeJsLogo } from "./app/_components/logos";
import { ThemeAwareImage } from "./components/theme-aware-image";
import { cn } from "./components/cn";

const iconAdder = (title?: string): JSX.Element | null => {
  if (title?.endsWith("turbo.json")) {
    const size = 14;
    return (
      <ThemeAwareImage
        light={{
          src: "/images/product-icons/repo-light-32x32.png",
          alt: "Turborepo logo",
          className: "grayscale",
          props: {
            src: "/images/product-icons/repo-light-32x32.png",
            alt: "Turborepo logo",
            width: size,
            height: size,
          },
        }}
        dark={{
          src: "/images/product-icons/repo-dark-32x32.png",
          alt: "Turborepo logo",
          className: "grayscale",
          props: {
            src: "/images/product-icons/repo-dark-32x32.png",
            alt: "Turborepo logo",
            width: size,
            height: size,
          },
        }}
      />
    );
  }

  if (title?.endsWith("package.json")) {
    return <NodeJsLogo className="grayscale" />;
  }
  return null;
};

export const mdxComponents: MDXComponents = {
  ...defaultComponents,
  h1: (props) => {
    const { className, ...rest } = props;

    return (
      <Heading
        className={cn("font-semibold text-center", className)}
        as="h1"
        {...rest}
      />
    );
  },
  h2: (props) => (
    <Heading className="scroll-mt-20 text-heading-24" as="h2" {...props} />
  ),
  h3: (props) => (
    <Heading className="scroll-mt-20 text-heading-20" as="h3" {...props} />
  ),
  h4: (props) => (
    <Heading className="text-lg scroll-mt-20" as="h4" {...props} />
  ),
  a: (props) => (
    <a className="text-[var(--ds-blue-900)] no-underline" {...props} />
  ),
  pre: ({ ref: _ref, title, ...props }) => {
    /* @ts-expect-error - Props are dynamically passed through MDX and include additional properties */
    const preIcon = props.icon as ReactNode;
    /* @ts-expect-error - Props are dynamically passed through MDX and include additional properties */
    const { icon: _icon, ...preProps } = props;

    if (!title) {
      throw new Error(
        'Code blocks must have titles. If you are creating a terminal, use "Terminal" for the title. Else, add a file path name.'
      );
    }

    return (
      <CodeBlock {...props} icon={iconAdder(title) ?? preIcon} title={title}>
        <Pre {...preProps} />
      </CodeBlock>
    );
  },
};
