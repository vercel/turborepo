CI runs `turbo run test` without a docs preview token. The `docs` workspace has no tests, but CI still builds it and fails because `DOCS_PREVIEW_TOKEN` is missing.

Fix the task setup so this command runs tests and the builds those tests need, including the UI library build before the web build. Test-less workspaces that are not needed by the tests must stay out of the run, even as new workspaces are added.

Keep the CI command unchanged. Don't add filters, special-case workspace names, or change the workspace scripts. A normal `turbo run build` must still include docs and require its preview token.
