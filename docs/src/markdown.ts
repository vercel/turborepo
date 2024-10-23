import fs from "fs/promises";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import rehypeRaw from "rehype-raw";
import { visit } from "unist-util-visit";
import GitHubSlugger from "github-slugger";
import matter from "gray-matter";

export interface Document {
  /** the Markdown file itself, without from-matter */
  content: string;

  /** the path to this markdown file */
  path: string;

  /** the headings found in this markdown file */
  headings: string[];

  frontMatter: {
    title: string;
    description: string;
  };
}

export type ErrorType = "link" | "hash" | "source" | "related";

export type LinkError = {
  type: ErrorType;
  href: string;
  doc: Document;
};

/** where to look for docs (.mdx files) */
const DOCS_PATH = ".";
const EXCLUDED_HASHES = ["top"];

/** These paths exist, just not in our Markdown files */
const EXCLUDED_PATHS = ["/api/remote-cache-spec", "/repo"];

const slugger = new GitHubSlugger();

/** Collect the paths of all .mdx files we care about */
const getAllMdxFilePaths = async (): Promise<string[]> => {
  const allFiles = await fs.readdir(DOCS_PATH, { recursive: true });
  return allFiles.filter((file) => file.endsWith(".mdx"));
};

// Returns the slugs of all headings in a tree
const getHeadingsFromMarkdownTree = (
  tree: ReturnType<typeof markdownProcessor.parse>
): string[] => {
  const headings: string[] = [];
  slugger.reset();

  visit(tree, "heading", (node) => {
    let headingText = "";
    // Account for headings with inline code blocks by concatenating the
    // text values of all children of a heading node.
    visit(node, (innerNode: any) => {
      if (innerNode.value) {
        headingText += innerNode.value;
      }
    });
    const slugified = slugger.slug(headingText);
    headings.push(slugified);
  });

  return headings;
};

/** Create a processor to parse MDX content */
const markdownProcessor = unified()
  .use(remarkParse)
  .use(remarkRehype)
  .use(rehypeRaw)
  .use(function compiler() {
    // A compiler is required, and we only need the AST, so we can
    // just return it.
    // @ts-ignore
    this.Compiler = function treeCompiler(tree) {
      return tree;
    };
  });

const filePathToUrl = (filePath: string): string =>
  filePath
    .replace("repo-docs", "/repo/docs")
    .replace("pack-docs", "/pack/docs")
    .replace(".mdx", "");

const validateFrontmatter = (path: string, data: Record<string, unknown>) => {
  if (!data.title) {
    throw new Error(`Document is missing a title: ${path}`);
  }
  if (!data.description) {
    throw new Error(`Document is missing a description: ${path}`);
  }
  return data as {
    title: string;
    description: string;
  };
};

/**
 * Create a map of documents with their paths as keys and
 * document content and metadata as values
 * The key varies between doc pages and error pages
 * error pages: `/docs/messages/example`
 * doc pages: `api/example`
 */
const prepareDocumentMapEntry = async (
  path: string
): Promise<[string, Document]> => {
  try {
    const mdxContent = await fs.readFile(path, "utf8");
    const { content, data } = matter(mdxContent);
    const frontMatter = validateFrontmatter(path, data);

    const tree = markdownProcessor.parse(content);
    const headings = getHeadingsFromMarkdownTree(tree);
    const normalizedUrlPath = filePathToUrl(path);

    return [normalizedUrlPath, { content, path, headings, frontMatter }];
  } catch (error) {
    throw new Error(`Error preparing document map for file ${path}: ${error}`);
  }
};

/** Checks if the links point to existing documents */
const validateInternalLink =
  (documentMap: Map<string, Document>) => (doc: Document, href: string) => {
    // /docs/api/example#heading -> ["/docs/api/example", "heading""]
    const [link, hash] = href.replace(DOCS_PATH, "").split("#", 2);

    if (EXCLUDED_PATHS.includes(link)) {
      return [];
    }

    let foundPage = documentMap.get(link);

    if (!foundPage) {
      foundPage = documentMap.get(`${link}/index`);
    }

    let errors: LinkError[] = [];

    if (!foundPage) {
      errors.push({
        type: "link",
        href,
        doc,
      });
    } else if (hash && !EXCLUDED_HASHES.includes(hash)) {
      // Check if the hash link points to an existing section within the document
      const hashFound = foundPage.headings.includes(hash);

      if (!hashFound) {
        errors.push({
          type: "hash",
          href,
          doc,
        });
      }
    }

    return errors;
  };

/** Checks if the hash links point to existing sections within the same document */
const validateHashLink = (doc: Document, href: string) => {
  const hashLink = href.replace("#", "");
  if (EXCLUDED_HASHES.includes(hashLink)) {
    return [];
  }

  if (doc.headings.includes(hashLink)) {
    return [];
  }

  let linkError: LinkError = {
    type: "hash",
    href,
    doc,
  };
  const { content, ...docWithoutContent } = doc;
  return [linkError];
};

/** Traverse the document tree and validate links */
const traverseTreeAndValidateLinks = (
  documentMap: Map<string, Document>,
  tree: unknown,
  doc: Document
): LinkError[] => {
  let errors: LinkError[] = [];

  try {
    visit(tree, (node: any) => {
      if (node.type === "element" && node.tagName === "a") {
        const href = node.properties.href;

        if (!href) {
          return;
        }

        if (href.startsWith("/")) {
          errors.push(...validateInternalLink(documentMap)(doc, href));
        } else if (href.startsWith("#")) {
          errors.push(...validateHashLink(doc, href));
        }
      }
    });
  } catch (error) {
    throw new Error(`Error traversing tree: ${error}`);
  }

  return errors;
};

/**
 * this function will look through all Mdx files and compile a list of `LinkError`s
 */
export const collectLinkErrors = async (): Promise<LinkError[]> => {
  const allMdxFilePaths = await getAllMdxFilePaths();

  const documentMap = new Map(
    await Promise.all(allMdxFilePaths.map(prepareDocumentMapEntry))
  );

  const reportsWithErrors = allMdxFilePaths.map(async (filePath) => {
    const doc = documentMap.get(filePathToUrl(filePath));
    if (!doc) {
      return null;
    }
    const vFile = await markdownProcessor.process(doc.content);
    const tree = vFile.result;
    const linkErrors = traverseTreeAndValidateLinks(documentMap, tree, doc);
    if (linkErrors.length > 0) {
      return linkErrors;
    }
    return null;
  });

  const results = await Promise.all(reportsWithErrors);
  const linkErrors = results.filter((report) => report !== null).flat();
  return linkErrors;
};
