import type { MDXComponents } from "mdx/types";
import defaultComponents from "fumadocs-ui/mdx";
import { Pre, CodeBlock } from "fumadocs-ui/components/codeblock";
import type { ReactNode, Ref } from "react";
import { NodeJsLogo, TurborepoLogo } from "./app/_components/logos";

const iconAdder = (title?: string): JSX.Element | null => {
  if (title?.endsWith("turbo.json")) {
    return <TurborepoLogo className="grayscale" />;
  }

  if (title?.endsWith("package.json")) {
    return <NodeJsLogo className="grayscale" />;
  }
  return null;
};

export const mdxComponents: MDXComponents = {
  ...defaultComponents,
  pre: ({
    ref: _ref,
    title,
    ...props
  }: {
    icon: ReactNode;
    ref: Ref<HTMLPreElement>;
    title?: string;
  }) => {
    const preIcon: ReactNode = props.icon;
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
