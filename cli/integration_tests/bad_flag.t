Setup
  $ . ${TESTDIR}/setup.sh

Bad flag should print misuse text
  $ ${TURBO} --bad-flag
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo
  error: Found argument '--bad-flag' which wasn't expected, or isn't valid in this context
  
    If you tried to supply '--bad-flag' as a value rather than a flag, use '-- --bad-flag'
  
  Usage: turbo [OPTIONS] [COMMAND]
  
  For more information try '--help'
  [1]
