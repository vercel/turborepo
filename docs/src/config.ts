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

export type ReportRow = {
  link: LinkError["href"];
  type: LinkError["type"];
  path: Document["path"];
};

export interface DocumentReport {
  doc: Document;
  errors: LinkError[];
}

export const DOCS_PATH = ".";
export const EXCLUDED_HASHES = ["top"];
