import { collectLinkErrors } from "./markdown";

/*
  This script validates internal links in /docs and /errors including internal,
  hash, source and related links. It does not validate external links.
  1. Collects all .mdx files in /docs.
  2. For each file, it extracts the content, metadata, and heading slugs.
  3. It creates a document map to efficiently lookup documents by path.
  4. It then traverses each document modified in the PR and...
     - Checks if each internal link (links starting with "/docs/") points
       to an existing document
     - Validates hash links (links starting with "#") against the list of
       headings in the current document.
     - Checks the source and related links found in the metadata of each
       document.
  5. Any broken links discovered during these checks are categorized and a
  comment is added to the PR.
*/

/** Main function that triggers link validation across .mdx files */
const validateAllInternalLinks = async (): Promise<void> => {
  let errorReports = await collectLinkErrors();
  if (errorReports.length === 0) {
    console.log("Link validation was successful.");
    return;
  }

  const reportRows = errorReports
    .map((linkError) => ({
      link: linkError.href,
      type: linkError.type,
      path: linkError.doc.path,
    }))
    .sort((a, b) => a.type.localeCompare(b.type));

  const plural = errorReports.length > 1;
  console.log(
    `Found ${plural ? "these" : "a"} broken link${
      plural ? "s" : ""
    } in the docs:`
  );
  console.table(reportRows);
  process.exit(1);
};

validateAllInternalLinks();
