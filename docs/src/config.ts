export interface Document {
  body: string;
  path: string;
  headings: string[];
  source?: string;
  related?: {
    links: string[];
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
