Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Bad flag should print misuse text
  $ ${TURBO} --bad-flag
   ERROR  unexpected argument '--bad-flag' found
  
    tip: to pass '--bad-flag' as a value, use '-- --bad-flag'
  
  Usage: turbo(\.exe)? .* (re)
  
  For more information, try '--help'.
  
  [1]

Bad flag with an implied run command should display run flags
  $ ${TURBO} build --bad-flag
   ERROR  unexpected argument '--bad-flag' found
  
    tip: to pass '--bad-flag' as a value, use '-- --bad-flag'
  
  Usage: turbo(\.exe)? .* (re)
  
  For more information, try '--help'.
  
  [1]

