# GitHub Action for Validating Documentation Links

This action ensures that internal links in `.mdx` files in the `/docs/` directory are valid. It runs on every pull request that includes changes to these files.

The action is triggered by the workflow defined in `.github/workflows/docs.yml`.

## Usage

pnpm check-links
