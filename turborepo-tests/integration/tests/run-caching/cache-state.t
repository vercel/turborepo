Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Run a build to get a local cache.
  $ ${TURBO} run build --output-logs=none
  \xe2\x80\xa2 Packages in scope: another, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s+[.0-9]+m?s  (re)
  

Do a dry run so we can see the state of the cache
  $ ${TURBO} run build --dry=json > dry.json

Get the hash of the my-app#build task, so we can inspect the cache
  $ HASH=$(cat dry.json | jq -r '.tasks | map(select(.taskId == "my-app#build")) | .[0].hash')
  $ duration=$(cat "node_modules/.cache/turbo/$HASH-meta.json" | jq .duration)
check that it exists
  $ echo $duration
  [0-9]+ (re)
should not be 0
  $ test $duration != 0

Validate that local cache is true in dry run
  $ cat dry.json | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].cache'
  {
    "local": true,
    "remote": false,
    "status": "HIT",
    "source": "LOCAL",
    "timeSaved": [0-9]+ (re)
  }
