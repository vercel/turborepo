import type { EnvWildcard } from "@turbo/types";

const reRegExpChar = /[\\^$.*+?()[\]{}|]/g;
const reHasRegExpChar = RegExp(reRegExpChar.source);
function escapeRegExp(string: string) {
  return string && reHasRegExpChar.test(string)
    ? string.replace(reRegExpChar, "\\$&")
    : string || "";
}

const wildcard = "*";
const wildcardEscape = "\\";
const regexWildcardSegment = ".*";

function wildcardToRegexPattern(pattern: string): string {
  const regexString: Array<string> = [];

  let previousIndex = 0;
  let previousRune: null | string = null;

  for (let i = 0; i < pattern.length; i++) {
    const char = pattern[i];
    if (char === wildcard) {
      if (previousRune === wildcardEscape) {
        // Found a literal *

        // Replace the trailing "\*" with just "*" before adding the segment.
        regexString.push(
          escapeRegExp(`${pattern.slice(previousIndex, i - 1)}*`)
        );
      } else {
        // Found a wildcard

        // Add in the static segment since the last wildcard. Can be zero length.
        regexString.push(escapeRegExp(pattern.slice(previousIndex, i)));

        // Add a dynamic segment if it isn't adjacent to another dynamic segment.
        if (regexString[regexString.length - 1] !== regexWildcardSegment) {
          regexString.push(regexWildcardSegment);
        }
      }

      // Advance the pointer.
      previousIndex = i + 1;
    }
    previousRune = char;
  }

  // Add the last static segment. Can be zero length.
  regexString.push(escapeRegExp(pattern.slice(previousIndex)));

  return regexString.join("");
}

interface Testable {
  test: (input: string) => boolean;
}

const NO_PATTERNS = {
  test(_: string): boolean {
    return false;
  },
};

export interface WildcardTests {
  inclusions: Testable;
  exclusions: Testable;
}

// wildcardTests returns a WildcardSet after processing wildcards against it.
export function wildcardTests(
  wildcardPatterns: Array<EnvWildcard>
): WildcardTests {
  const includePatterns: Array<string> = [];
  const excludePatterns: Array<string> = [];

  wildcardPatterns.forEach((wildcardPattern) => {
    const isExclude = wildcardPattern.startsWith("!");
    const isLiteralLeadingExclamation = wildcardPattern.startsWith("\\!");

    if (isExclude) {
      const excludePattern = wildcardToRegexPattern(wildcardPattern.slice(1));
      excludePatterns.push(excludePattern);
    } else if (isLiteralLeadingExclamation) {
      const includePattern = wildcardToRegexPattern(wildcardPattern.slice(1));
      includePatterns.push(includePattern);
    } else {
      const includePattern = wildcardToRegexPattern(wildcardPattern);
      includePatterns.push(includePattern);
    }
  });

  // Set some defaults.
  let inclusions = NO_PATTERNS;
  let exclusions = NO_PATTERNS;

  // Override if they're not empty.
  if (includePatterns.length > 0) {
    inclusions = new RegExp(`^(${includePatterns.join("|")})$`);
  }
  if (excludePatterns.length > 0) {
    exclusions = new RegExp(`^(${excludePatterns.join("|")})$`);
  }

  return {
    inclusions,
    exclusions,
  };
}
