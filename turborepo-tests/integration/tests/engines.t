Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
  $ jq '.engines = {"node": ">=12"}' package.json > package.json.new
  $ mv package.json.new package.json

Check a hash
  $ ${TURBO} build --dry=json --filter=my-app | jq ".tasks.[0].hash"
  "e1b18c14c735bc25"
Change engines
  $ jq '.engines = {"node": ">=16"}' package.json > package.json.new
  $ mv package.json.new package.json

Verify hash has changed
  $ ${TURBO} build --dry=json --filter=my-app | jq ".tasks.[0].hash"
  "639b83eff0f48891"

Verify engines are part of global cache inputs
  $ ${TURBO} build --dry=json | jq '.globalCacheInputs.engines'
  {
    "node": ">=16"
  }
