Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

Create test files that should be included via prune.includes
  $ echo "# Test README" > README.md
  $ echo "MIT License" > LICENSE
  $ mkdir -p docs
  $ echo "# Documentation" > docs/guide.md
  $ mkdir -p config
  $ echo '{"env": "production"}' > config/prod.json
  $ echo '{"env": "local"}' > config/local.json
  $ mkdir -p shared-assets
  $ echo "asset content" > shared-assets/logo.svg

Test basic custom includes in root turbo.json
  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["README.md", "LICENSE"]
  >   }
  > }
  > EOF

  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify basic includes are copied to all destinations
  $ test -f out/full/README.md
  $ test -f out/full/LICENSE
  $ test -f out/json/README.md
  $ test -f out/json/LICENSE
  $ test -f out/README.md
  $ test -f out/LICENSE

Test glob patterns with custom includes
  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["docs/**", "*.md"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify glob patterns work
  $ test -d out/full/docs
  $ test -f out/full/docs/guide.md
  $ test -f out/full/README.md

Test exclusion patterns with negation at root level
  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["config/**", "!config/local.json"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify included config files are copied
  $ test -f out/full/config/prod.json

Verify excluded config file is NOT copied
  $ test ! -f out/full/config/local.json

Verify config directory exists but excluded file is not in it
  $ ls out/full/config
  prod.json

Verify exclusions work in all output directories
  $ test -f out/config/prod.json
  $ test ! -f out/config/local.json
  $ test -f out/json/config/prod.json
  $ test ! -f out/json/config/local.json

Test multiple exclusion patterns
  $ mkdir -p secrets
  $ echo "secret1" > secrets/api-key.txt
  $ echo "secret2" > secrets/password.txt
  $ echo "not secret" > secrets/readme.txt
  $ mkdir -p logs
  $ echo "log data" > logs/debug.log
  $ echo "more logs" > logs/error.log

  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["secrets/**", "logs/**", "!secrets/api-key.txt", "!secrets/password.txt", "!logs/*.log"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify only non-excluded files are copied
  $ test -f out/full/secrets/readme.txt
  $ test ! -f out/full/secrets/api-key.txt
  $ test ! -f out/full/secrets/password.txt
  $ test ! -f out/full/logs/debug.log
  $ test ! -f out/full/logs/error.log

Test workspace-level custom includes with workspace-relative paths
  $ mkdir -p apps/web/docs
  $ echo "# Web App Docs" > apps/web/docs/README.md
  $ echo "# Web README" > apps/web/README.md
  $ echo "module.exports = {}" > apps/web/next.config.js
  $ echo "module.exports = {}" > apps/web/tailwind.config.js

Reset root turbo.json to have no custom includes
  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json"
  > }
  > EOF

Add workspace-level turbo.json with custom includes
  $ cat > apps/web/turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["README.md", "docs/**", "*.config.js"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify workspace-relative includes are properly prefixed and copied
  $ test -f out/full/apps/web/README.md
  $ test -d out/full/apps/web/docs
  $ test -f out/full/apps/web/docs/README.md
  $ test -f out/full/apps/web/next.config.js
  $ test -f out/full/apps/web/tailwind.config.js
  $ cat out/full/apps/web/README.md
  # Web README

Verify workspace files are also in docker output directories
  $ test -f out/json/apps/web/README.md
  $ test -f out/json/apps/web/next.config.js

Verify workspace files are NOT copied to repo root (only in apps/web/)
  $ test ! -f out/full/README.md
  $ test ! -f out/full/next.config.js

Test workspace-level includes with $TURBO_ROOT$ (repo-relative paths)
  $ cat > apps/web/turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["\$TURBO_ROOT\$/shared-assets/**"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify $TURBO_ROOT$ patterns work from workspace configs
  $ test -d out/full/shared-assets
  $ test -f out/full/shared-assets/logo.svg

Test mixed workspace and root includes
  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["LICENSE"]
  >   }
  > }
  > EOF

  $ cat > apps/web/turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["README.md"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web --docker
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify both root and workspace includes are applied
  $ test -f out/full/LICENSE
  $ test -f out/full/apps/web/README.md

Test without docker mode
  $ cat > turbo.json << EOF
  > {
  >   "\$schema": "https://turbo.build/schema.json",
  >   "prune": {
  >     "includes": ["README.md"]
  >   }
  > }
  > EOF

  $ rm -rf out
  $ ${TURBO} prune web
  Generating pruned monorepo for web in .*out (re)
   - Added shared
   - Added util
   - Added web

Verify files are copied when not in docker mode
  $ test -f out/README.md
  $ test ! -d out/json
  $ test ! -d out/full
