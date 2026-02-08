Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh --no-install

Configure baseline values in turbo.json
  $ cat > turbo.json << 'EOF'
  > {
  >   "tasks": {
  >     "build": {
  >       "outputs": []
  >     }
  >   },
  >   "remoteCache": {
  >     "apiUrl": "https://turbo-json-api.example.com",
  >     "loginUrl": "https://turbo-json-login.example.com",
  >     "teamSlug": "team-from-turbo-json",
  >     "teamId": "id-from-turbo-json",
  >     "timeout": 111,
  >     "uploadTimeout": 222,
  >     "preflight": false
  >   },
  >   "daemon": true,
  >   "envMode": "strict",
  >   "cacheDir": "cache-from-turbo-json",
  >   "concurrency": "3"
  > }
  > EOF

Baseline: values are read from turbo.json
  $ ${TURBO} config | jq -e '.apiUrl == "https://turbo-json-api.example.com" and .loginUrl == "https://turbo-json-login.example.com" and .teamSlug == "team-from-turbo-json" and .teamId == "id-from-turbo-json" and .timeout == 111 and .uploadTimeout == 222 and .preflight == false and .daemon == true and .envMode == "strict" and .cacheDir == "cache-from-turbo-json" and .concurrency == "3"' > /dev/null

TURBO_ROOT_TURBO_JSON is respected, and --root-turbo-json takes precedence
  $ jq '.daemon = false | .concurrency = "99"' turbo.json > turbo-alt.json
  $ TURBO_ROOT_TURBO_JSON=turbo-alt.json ${TURBO} config | jq -e '.daemon == false and .concurrency == "99"' > /dev/null
  $ TURBO_ROOT_TURBO_JSON=turbo-alt.json ${TURBO} --root-turbo-json=turbo.json config | jq -e '.daemon == true and .concurrency == "3"' > /dev/null

Global config overrides turbo.json
  $ CONFIG_DIR="$(mktemp -d)"
  $ export TURBO_CONFIG_DIR_PATH="$CONFIG_DIR"
  $ mkdir -p "$CONFIG_DIR/turborepo"
  $ cat > "$CONFIG_DIR/turborepo/config.json" << 'EOF'
  > {
  >   "apiUrl": "https://global-api.example.com",
  >   "loginUrl": "https://global-login.example.com",
  >   "teamSlug": "team-from-global",
  >   "teamId": "id-from-global",
  >   "timeout": 333,
  >   "uploadTimeout": 444,
  >   "preflight": true,
  >   "daemon": false,
  >   "envMode": "loose",
  >   "cacheDir": "cache-from-global",
  >   "concurrency": "7"
  > }
  > EOF
  $ ${TURBO} config | jq -e '.apiUrl == "https://global-api.example.com" and .loginUrl == "https://global-login.example.com" and .teamSlug == "team-from-global" and .teamId == "id-from-global" and .timeout == 333 and .uploadTimeout == 444 and .preflight == true and .daemon == false and .envMode == "loose" and .cacheDir == "cache-from-global" and .concurrency == "7"' > /dev/null

Local config overrides global config
  $ mkdir -p .turbo
  $ cat > .turbo/config.json << 'EOF'
  > {
  >   "apiUrl": "https://local-api.example.com",
  >   "loginUrl": "https://local-login.example.com",
  >   "teamSlug": "team-from-local",
  >   "teamId": "id-from-local",
  >   "timeout": 444,
  >   "uploadTimeout": 555,
  >   "preflight": false,
  >   "daemon": true,
  >   "envMode": "strict",
  >   "cacheDir": "cache-from-local",
  >   "concurrency": "9"
  > }
  > EOF
  $ ${TURBO} config | jq -e '.apiUrl == "https://local-api.example.com" and .loginUrl == "https://local-login.example.com" and .teamSlug == "team-from-local" and .teamId == "id-from-local" and .timeout == 444 and .uploadTimeout == 555 and .preflight == false and .daemon == true and .envMode == "strict" and .cacheDir == "cache-from-local" and .concurrency == "9"' > /dev/null

Environment overrides local config
  $ TURBO_API=https://env-api.example.com TURBO_LOGIN=https://env-login.example.com TURBO_TEAM=team-from-env TURBO_TEAMID=id-from-env TURBO_REMOTE_CACHE_TIMEOUT=555 TURBO_REMOTE_CACHE_UPLOAD_TIMEOUT=666 TURBO_PREFLIGHT=true TURBO_DAEMON=false TURBO_ENV_MODE=loose TURBO_CACHE_DIR=cache-from-env TURBO_CONCURRENCY=11 ${TURBO} config | jq -e '.apiUrl == "https://env-api.example.com" and .loginUrl == "https://env-login.example.com" and .teamSlug == "team-from-env" and .teamId == "id-from-env" and .timeout == 555 and .uploadTimeout == 666 and .preflight == true and .daemon == false and .envMode == "loose" and .cacheDir == "cache-from-env" and .concurrency == "11"' > /dev/null

CLI overrides environment
  $ TURBO_API=https://env-api.example.com TURBO_LOGIN=https://env-login.example.com TURBO_TEAM=team-from-env TURBO_TEAMID=id-from-env TURBO_REMOTE_CACHE_TIMEOUT=555 TURBO_REMOTE_CACHE_UPLOAD_TIMEOUT=666 TURBO_PREFLIGHT=false TURBO_DAEMON=false TURBO_ENV_MODE=strict TURBO_CACHE_DIR=cache-from-env TURBO_CONCURRENCY=11 ${TURBO} --api=https://flag-api.example.com --login=https://flag-login.example.com --team=team-from-flag --remote-cache-timeout=777 --preflight --daemon --env-mode=loose --cache-dir=cache-from-flag --concurrency=13 config | jq -e '.apiUrl == "https://flag-api.example.com" and .loginUrl == "https://flag-login.example.com" and .teamSlug == "team-from-flag" and .teamId == "id-from-env" and .timeout == 777 and .uploadTimeout == 666 and .preflight == true and .daemon == true and .envMode == "loose" and .cacheDir == "cache-from-flag" and .concurrency == "13"' > /dev/null
