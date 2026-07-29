#!/usr/bin/env bash
# Open Phase 3 + Phase 4 stacked PRs (requires gh auth with PR create).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
REPO="${REPO:-vercel/turborepo}"

gh pr create --repo "$REPO" --draft \
  --base main \
  --head shew/turbo-5811-migrate-external-package-queries \
  --title "refactor: Migrate external package queries to resolution knowledge" \
  --body-file "$ROOT/stack-pr-bodies/01-5811.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5811-migrate-external-package-queries \
  --head shew/turbo-5812-migrate-prune-external-closures \
  --title "refactor: Migrate prune lockfile keys to resolution identities" \
  --body-file "$ROOT/stack-pr-bodies/02-5812.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5812-migrate-prune-external-closures \
  --head shew/turbo-5813-migrate-global-external-hash-inputs \
  --title "refactor: Migrate global hash inputs to resolution knowledge" \
  --body-file "$ROOT/stack-pr-bodies/03-5813.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5813-migrate-global-external-hash-inputs \
  --head shew/turbo-5814-delete-legacy-external-resolution-state \
  --title "refactor: Delete legacy external-resolution PackageInfo state" \
  --body-file "$ROOT/stack-pr-bodies/04-5814.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5814-delete-legacy-external-resolution-state \
  --head shew/turbo-5825-remove-external-declaration-compatibility-paths \
  --title "refactor: Remove external declaration compatibility paths" \
  --body-file "$ROOT/stack-pr-bodies/05-5825.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5825-remove-external-declaration-compatibility-paths \
  --head shew/turbo-5816-produce-immutable-native-task-and-command-knowledge \
  --title "refactor: Produce immutable native task and command knowledge" \
  --body-file "$ROOT/stack-pr-bodies/06-5816.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5816-produce-immutable-native-task-and-command-knowledge \
  --head shew/turbo-5817-migrate-native-task-registration-and-suggestions \
  --title "refactor: Migrate native task registration and suggestions" \
  --body-file "$ROOT/stack-pr-bodies/07-5817.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5817-migrate-native-task-registration-and-suggestions \
  --head shew/turbo-5818-migrate-turbo-json-native-task-synthesis \
  --title "refactor: Migrate turbo-json native task synthesis" \
  --body-file "$ROOT/stack-pr-bodies/08-5818.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5818-migrate-turbo-json-native-task-synthesis \
  --head shew/turbo-5819-migrate-persistent-and-recursive-task-validation \
  --title "refactor: Migrate persistent and recursive task validation" \
  --body-file "$ROOT/stack-pr-bodies/09-5819.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5819-migrate-persistent-and-recursive-task-validation \
  --head shew/turbo-5820-migrate-native-task-definition-precedence \
  --title "refactor: Migrate native task definition precedence" \
  --body-file "$ROOT/stack-pr-bodies/10-5820.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5820-migrate-native-task-definition-precedence \
  --head shew/turbo-5821-migrate-engine-native-command-planning \
  --title "refactor: Migrate engine native command planning" \
  --body-file "$ROOT/stack-pr-bodies/11-5821.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5821-migrate-engine-native-command-planning \
  --head shew/turbo-5822-migrate-executor-native-command-resolution \
  --title "refactor: Migrate executor native command resolution" \
  --body-file "$ROOT/stack-pr-bodies/12-5822.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5822-migrate-executor-native-command-resolution \
  --head shew/turbo-5823-migrate-native-task-query-devtools-and-lsp-views \
  --title "refactor: Migrate native task query, devtools, and LSP views" \
  --body-file "$ROOT/stack-pr-bodies/13-5823.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5823-migrate-native-task-query-devtools-and-lsp-views \
  --head shew/turbo-5824-migrate-command-summaries-and-delete-legacy-task-paths \
  --title "refactor: Migrate command summaries and delete legacy task paths" \
  --body-file "$ROOT/stack-pr-bodies/14-5824.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5824-migrate-command-summaries-and-delete-legacy-task-paths \
  --head shew/turbo-5826-produce-immutable-task-contract-knowledge \
  --title "refactor: Produce immutable task-contract knowledge" \
  --body-file "$ROOT/stack-pr-bodies/15-5826.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5826-produce-immutable-task-contract-knowledge \
  --head shew/turbo-5827-migrate-engine-task-contract-composition \
  --title "refactor: Migrate engine task-contract composition" \
  --body-file "$ROOT/stack-pr-bodies/16-5827.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5827-migrate-engine-task-contract-composition \
  --head shew/turbo-5828-migrate-hashing-and-cache-to-task-contracts \
  --title "refactor: Migrate hashing engines to task contracts" \
  --body-file "$ROOT/stack-pr-bodies/17-5828.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5828-migrate-hashing-and-cache-to-task-contracts \
  --head shew/turbo-5829-migrate-dry-run-summary-contracts-and-delete-js-io-callbacks \
  --title "refactor: Exclude JavaScript from toolchain task-I/O dispatch" \
  --body-file "$ROOT/stack-pr-bodies/18-5829.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5829-migrate-dry-run-summary-contracts-and-delete-js-io-callbacks \
  --head shew/turbo-5830-produce-immutable-change-knowledge \
  --title "refactor: Produce immutable change knowledge" \
  --body-file "$ROOT/stack-pr-bodies/19-5830.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5830-produce-immutable-change-knowledge \
  --head shew/turbo-5832-migrate-watcher-classification-to-change-knowledge \
  --title "refactor: Migrate watcher classification to change knowledge" \
  --body-file "$ROOT/stack-pr-bodies/20-5832.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5832-migrate-watcher-classification-to-change-knowledge \
  --head shew/turbo-5831-delete-js-only-watcher-reconstruction-paths \
  --title "refactor: Delete JS lockfile probes from change classification" \
  --body-file "$ROOT/stack-pr-bodies/21-5831.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5831-delete-js-only-watcher-reconstruction-paths \
  --head shew/turbo-5833-extract-javascript-prune-rendering-pure-functions \
  --title "refactor: Extract JavaScript prune rendering pure functions" \
  --body-file "$ROOT/stack-pr-bodies/22-5833.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5833-extract-javascript-prune-rendering-pure-functions \
  --head shew/turbo-5834-separate-prune-closure-and-layout-from-js-rendering \
  --title "refactor: Separate prune closure and layout from JS rendering" \
  --body-file "$ROOT/stack-pr-bodies/23-5834.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5834-separate-prune-closure-and-layout-from-js-rendering \
  --head shew/turbo-5835-add-prune-golden-fixtures-for-retained-files-and-layers \
  --title "test: Add prune golden fixtures for retained files and layers" \
  --body-file "$ROOT/stack-pr-bodies/24-5835.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5835-add-prune-golden-fixtures-for-retained-files-and-layers \
  --head shew/turbo-5836-delete-js-format-interpretation-from-prune-orchestration \
  --title "refactor: Delete JS format interpretation from prune orchestration" \
  --body-file "$ROOT/stack-pr-bodies/25-5836.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5836-delete-js-format-interpretation-from-prune-orchestration \
  --head shew/turbo-5837-audit-and-close-remaining-js-knowledge-consumer-reads \
  --title "refactor: Audit and close remaining JS knowledge consumer reads" \
  --body-file "$ROOT/stack-pr-bodies/26-5837.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5837-audit-and-close-remaining-js-knowledge-consumer-reads \
  --head shew/turbo-5838-migrate-mfe-dependency-detection-off-packageinfo \
  --title "refactor: Migrate MFE dependency detection off PackageInfo" \
  --body-file "$ROOT/stack-pr-bodies/27-5838.md"

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5838-migrate-mfe-dependency-detection-off-packageinfo \
  --head shew/turbo-5839-migrate-prune-peer-helpers-off-packagejson \
  --title "refactor: Migrate prune peer helpers off PackageJson" \
  --body-file "$ROOT/stack-pr-bodies/28-5839.md"

echo "Opened stack through TURBO-5839."
