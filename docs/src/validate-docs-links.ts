import fs from "fs/promises";
import path from "path";
import unified from "unified";
import markdown from "remark-parse";
import remarkToRehype from "remark-rehype";
import raw from "rehype-raw";
import visit from "unist-util-visit";
import GithubSlugger from "github-slugger";
import matter from "gray-matter";
import {
  COMMENT_TAG,
  createComment,
  findBotComment,
  updateCheckStatus,
  updateComment,
  setFailed,
  sha,
} from "./github";

/**
 * This script validates internal links in /docs and /errors including internal,
 * hash, source and related links. It does not validate external links.
 * 1. Collects all .mdx files in /docs.
 * 2. For each file, it extracts the content, metadata, and heading slugs.
 * 3. It creates a document map to efficiently lookup documents by path.
 * 4. It then traverses each document modified in the PR and...
 *    - Checks if each internal link (links starting with "/docs/") points
 *      to an existing document
 *    - Validates hash links (links starting with "#") against the list of
 *      headings in the current document.
 *    - Checks the source and related links found in the metadata of each
 *      document.
 * 5. Any broken links discovered during these checks are categorized and a
 * comment is added to the PR.
 */

interface Document {
  body: string;
  path: string;
  headings: string[];
  source?: string;
  related?: {
    links: string[];
  };
}

interface Errors {
  doc: Document;
  link: string[];
  hash: string[];
  source: string[];
  related: string[];
}

type ErrorType = Exclude<keyof Errors, "doc">;

const DOCS_PATH = ".";
const EXCLUDED_HASHES = ["top"];

const slugger = new GithubSlugger();

// Collect the paths of all .mdx files in the passed directories
async function getAllMdxFilePaths(
  directoriesToScan: string[],
  fileList: string[] = []
): Promise<string[]> {
  for (const dir of directoriesToScan) {
    const dirPath = path.join(".", dir);
    const files = await fs.readdir(dirPath);
    for (const file of files) {
      const filePath = path.join(dirPath, file);
      const stats = await fs.stat(filePath);
      if (stats.isDirectory()) {
        fileList = await getAllMdxFilePaths([filePath], fileList);
      } else if (path.extname(file) === ".mdx") {
        fileList.push(filePath);
      }
    }
  }

  return fileList;
}

// Returns the slugs of all headings in a tree
function getHeadingsFromMarkdownTree(
  tree: ReturnType<typeof markdownProcessor.parse>
): string[] {
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
    headings.push(slugger.slug(headingText));
  });

  return headings;
}

// Create a processor to parse MDX content
const markdownProcessor = unified()
  .use(markdown)
  .use(remarkToRehype, { allowDangerousHTML: true })
  .use(raw)
  .use(function compiler() {
    // A compiler is required, and we only need the AST, so we can
    // just return it.
    // @ts-ignore
    this.Compiler = function treeCompiler(tree) {
      return tree;
    };
  });

function normalizePath(filePath: string): string {
  const normalized = filePath
    .replace("repo-docs", "/repo/docs")
    .replace("pack-docs", "/pack/docs")
    .replace(".mdx", "");

  return normalized;
}

// use Map for faster lookup
let documentMap: Map<string, Document>;

// Create a map of documents with their paths as keys and
// document content and metadata as values
// The key varies between doc pages and error pages
// error pages: `/docs/messages/example`
// doc pages: `api/example`
async function prepareDocumentMapEntry(
  filePath: string
): Promise<[string, Document]> {
  try {
    const mdxContent = await fs.readFile(filePath, "utf8");
    const { content, data } = matter(mdxContent);
    const tree = markdownProcessor.parse(content);
    const headings = getHeadingsFromMarkdownTree(tree);
    const normalizedUrlPath = normalizePath(filePath);

    return [
      normalizedUrlPath,
      { body: content, path: filePath, headings, ...data },
    ];
  } catch (error) {
    setFailed(`Error preparing document map for file ${filePath}: ${error}`);
    return ["", {} as Document];
  }
}

// Checks if the links point to existing documents
function validateInternalLink(errors: Errors, href: string): void {
  // /docs/api/example#heading -> ["api/example", "heading""]
  const [link, hash] = href.replace(DOCS_PATH, "").split("#", 2);

  // These paths exist, just not in our Markdown files
  const ignorePaths = ["/api/remote-cache-spec", "/repo"];
  if (ignorePaths.includes(link)) {
    return;
  }

  let foundPage = documentMap.get(link);

  if (!foundPage) {
    foundPage = documentMap.get(`${link}/index`);
  }

  if (!foundPage) {
    errors.link.push(href);
  } else if (hash && !EXCLUDED_HASHES.includes(hash)) {
    // Check if the hash link points to an existing section within the document
    const hashFound = foundPage.headings.includes(hash);

    if (!hashFound) {
      errors.hash.push(href);
    }
  }
}

// Checks if the hash links point to existing sections within the same document
function validateHashLink(errors: Errors, href: string, doc: Document): void {
  const hashLink = href.replace("#", "");

  if (!EXCLUDED_HASHES.includes(hashLink) && !doc.headings.includes(hashLink)) {
    errors.hash.push(href);
  }
}

// Traverse the document tree and validate links
function traverseTreeAndValidateLinks(tree: any, doc: Document): Errors {
  const errors: Errors = {
    doc,
    link: [],
    hash: [],
    source: [],
    related: [],
  };

  try {
    visit(tree, (node: any) => {
      if (node.type === "element" && node.tagName === "a") {
        const href = node.properties.href;

        if (!href) return;

        if (href.startsWith("/")) {
          validateInternalLink(errors, href);
        } else if (href.startsWith("#")) {
          validateHashLink(errors, href, doc);
        }
      }
    });
  } catch (error) {
    setFailed("Error traversing tree: " + error);
  }

  return errors;
}

const formatTableRow = (
  link: string,
  errorType: ErrorType,
  rawDocPath: string
) => {
  const docPath = rawDocPath.replace("../../../", "");

  return `| ${link} | ${errorType} | [/${docPath}](https://github.com/vercel/turborepo/blob/${sha}/${docPath}) | \n`;
};

// Main function that triggers link validation across .mdx files
async function validateAllInternalLinks(): Promise<void> {
  try {
    const allMdxFilePaths = await getAllMdxFilePaths([DOCS_PATH]);

    documentMap = new Map(
      await Promise.all(allMdxFilePaths.map(prepareDocumentMapEntry))
    );

    const docProcessingPromises = allMdxFilePaths.map(async (filePath) => {
      const doc = documentMap.get(normalizePath(filePath));
      if (doc) {
        const tree = (await markdownProcessor.process(doc.body)).contents;
        return traverseTreeAndValidateLinks(tree, doc);
      } else {
        return {
          doc: {} as Document,
          link: [],
          hash: [],
          source: [],
          related: [],
        } as Errors;
      }
    });

    const allErrors = await Promise.all(docProcessingPromises);

    let errorsExist = false;

    let errorRows: string[] = [];

    const errorTypes: ErrorType[] = ["link", "hash", "source", "related"];
    allErrors.forEach((errors) => {
      const {
        doc: { path: docPath },
      } = errors;

      errorTypes.forEach((errorType) => {
        if (errors[errorType].length > 0) {
          errorsExist = true;
          errors[errorType].forEach((link) => {
            errorRows.push(formatTableRow(link, errorType, docPath));
          });
        }
      });
    });

    const errorComment = [
      "Hi there :wave:\n\nIt looks like this PR introduces broken links to the docs, please take a moment to fix them before merging:\n\n| Broken link | Type | File | \n| ----------- | ----------- | ----------- | \n",
      ...errorRows,
      "\nThank you :pray:",
    ].join("");

    let commentUrl;
    let botComment;
    let comment;

    botComment = await findBotComment();

    if (errorsExist) {
      comment = `${COMMENT_TAG}\n${errorComment}`;
      if (botComment) {
        commentUrl = await updateComment(comment, botComment);
      } else {
        commentUrl = await createComment(comment);
      }

      const errorTableData = allErrors.flatMap((errors) => {
        const { doc } = errors;

        return errorTypes.flatMap((errorType) =>
          errors[errorType].map((link) => ({
            docPath: doc.path,
            errorType,
            link,
          }))
        );
      });

      console.log("This PR introduces broken links to the docs:");
      console.table(errorTableData, ["link", "type", "docPath"]);
      process.exit(1);
    } else if (botComment) {
      const comment = `${COMMENT_TAG}\nAll broken links are now fixed, thank you!`;
      commentUrl = await updateComment(comment, botComment);
    }

    try {
      await updateCheckStatus(errorsExist, commentUrl);
    } catch (error) {
      setFailed("Failed to create Github check: " + error);
    }
  } catch (error) {
    setFailed("Error validating internal links: " + error);
  }
}

validateAllInternalLinks();
