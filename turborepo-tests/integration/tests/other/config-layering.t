Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/10-too-many --no-install

Set baseline in turbo.json: concurrency=1
  $ jq '.concurrency = "1"' turbo.json > turbo.json.tmp && mv turbo.json.tmp turbo.json
  $ ${TURBO} run build > turbo-layer.log 2>&1
  [1]
  $ grep --quiet "concurrency of 1" turbo-layer.log

TURBO_ROOT_TURBO_JSON is respected, and --root-turbo-json takes precedence over it
  $ jq '.concurrency = "2"' turbo.json > turbo-alt.json
  $ TURBO_ROOT_TURBO_JSON=turbo-alt.json ${TURBO} run build > root-env.log 2>&1
  [1]
  $ grep --quiet "concurrency of 2" root-env.log
  $ TURBO_ROOT_TURBO_JSON=turbo-alt.json ${TURBO} run build --root-turbo-json=turbo.json > root-flag.log 2>&1
  [1]
  $ grep --quiet "concurrency of 1" root-flag.log

Global config overrides turbo.json
  $ CONFIG_DIR="$(mktemp -d)"
  $ export TURBO_CONFIG_DIR_PATH="$CONFIG_DIR"
  $ mkdir -p "$CONFIG_DIR/turborepo"
  $ cat > "$CONFIG_DIR/turborepo/config.json" << 'EOF'
  > {
  >   "concurrency": "2"
  > }
  > EOF
  $ ${TURBO} run build > global-layer.log 2>&1
  [1]
  $ grep --quiet "concurrency of 2" global-layer.log

Local config overrides global config
  $ mkdir -p .turbo
  $ cat > .turbo/config.json << 'EOF'
  > {
  >   "concurrency": "3"
  > }
  > EOF
  $ ${TURBO} run build > local-layer.log 2>&1
  $ grep -E "2 successful, 2 total" local-layer.log
   Tasks:    2 successful, 2 total

Environment overrides local config
  $ TURBO_CONCURRENCY=1 ${TURBO} run build > env-layer.log 2>&1
  [1]
  $ grep --quiet "concurrency of 1" env-layer.log

CLI flag overrides environment variable
  $ TURBO_CONCURRENCY=1 ${TURBO} run build --concurrency=3 > cli-layer.log 2>&1
  $ grep -E "2 successful, 2 total" cli-layer.log
   Tasks:    2 successful, 2 total
