Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
Clear name field
  $ jq '.name = ""' apps/my-app/package.json > package.json.new
  $ mv package.json.new apps/my-app/package.json
Build should fail due to missing name field
  $ ${TURBO} build 1> ERR
  [1]
  $ grep -F --quiet 'x package.json must have a name field:' ERR

Add a trailing comma
  $ echo "{ \"name\": \"foobar\", }" > package.json.new
  $ mv package.json.new apps/my-app/package.json
Build should fail due to trailing comma (sed replaces square brackets with parentheses)
  $ ${TURBO} build 2>&1 | sed  's/\[\([^]]*\)\]/\(\1)/g'
  package_json_parse_error
  
    x unable to parse package.json
  
  Error:   x Expected a property but instead found '}'.
     ,-\(.*package.json:1:1\) (re)
   1 | { "name": "foobar", }
     :                     ^
     `----
  
