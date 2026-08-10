import { CodeBlock } from "@vercel/geistdocs/components/code-block";
import { geistShikiTheme } from "@vercel/geistdocs/shiki-theme";
import { highlight } from "fumadocs-core/highlight";
import type { ComponentProps } from "react";
import type { BundledLanguage } from "shiki";

type HighlightedCodeProps = {
  code: string;
  lang: BundledLanguage;
  caption: string;
};

export const HighlightedCode = async ({
  code,
  lang,
  caption,
}: HighlightedCodeProps) => {
  const rendered = await highlight(code, {
    lang,
    engine: "js",
    theme: geistShikiTheme,
    components: {
      pre: ({ children, className, style }: ComponentProps<"pre">) => (
        <CodeBlock className={className} style={style}>
          {children}
        </CodeBlock>
      ),
    },
  });

  return (
    <div className="flex h-full flex-col">
      <div className="*:mb-0 *:h-full flex-1">{rendered}</div>
      <span className="mt-2 block text-copy-14 text-gray-900">{caption}</span>
    </div>
  );
};
