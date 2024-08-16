Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
Clear name field
  $ jq '.name = ""' apps/my-app/package.json > package.json.new
  $ mv apps/my-app/package.json apps/my-app/package.json.old
  $ mv package.json.new apps/my-app/package.json
Build should fail due to missing name field
  $ ${TURBO} build 2> ERR
  [1]
  $ grep -F --quiet 'x package.json must have a name field:' ERR

Restore name field
  $ mv apps/my-app/package.json.old apps/my-app/package.json

Clear add invalid packageManager field
  $ jq '.packageManager = "bower@8.19.4"' package.json > package.json.new
  $ mv package.json.new package.json

Build should fail due to invalid packageManager field (sed removes the square brackets)
  $ ${TURBO} build 2> ERR
  [1]
  $ sed  's/\[\([^]]*\)\]/\\1/g' < ERR
  invalid_package_manager_field
  
    x could not resolve workspaces
    `-> could not parse the packageManager field in package.json, expected to
        match regular expression (?P<manager>bun|npm|pnpm|yarn)@(?P<version>\d+
        \.\d+\.\d+(-.+)?)
     ,-\1
   5 |   },
   6 |   "packageManager": "bower@8.19.4",
     :                     ^^^^^^^|^^^^^^
     :                            `-- invalid `packageManager` field
   7 |   "workspaces": [
     `----
  

Add invalid packageManager field that passes the regex.
  $ jq '.packageManager = "npm@0.3.211111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111"' package.json > package.json.new
  $ mv package.json.new package.json

  $ ${TURBO} build 2> ERR
  [1]
  $ sed  's/\[\([^]]*\)\]/\(\1)/g' < ERR
  invalid_semantic_version
  
    x could not resolve workspaces
    `-> invalid semantic version: Failed to parse an integer component of a
        semver string: number too large to fit in target type
     ,-\(.*package.json:5:1\) (re)
   5 |   },
   6 |   "packageManager": "npm@0.3.211111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
     :                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^|^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
     :                                                                                                                                             `-- version found here
   7 |   "workspaces": [
     `----
  
Restore packageManager field
  $ jq '.packageManager = "npm@8.19.4"' package.json > package.json.new
  $ mv package.json.new package.json

Add a trailing comma
  $ echo "{ \"name\": \"foobar\", }" > package.json.new
  $ mv package.json.new apps/my-app/package.json
Build should fail due to trailing comma (sed replaces square brackets with parentheses)
  $ ${TURBO} build 2> ERR
  [1]
  $ sed  's/\[\([^]]*\)\]/\(\1)/g' < ERR
  package_json_parse_error
  
    x unable to parse package.json
  
  Error:   x Expected a property but instead found '}'.
     ,-\(.*package.json:1:1\) (re)
   1 | { "name": "foobar", }
     :                     ^
     `----
  



