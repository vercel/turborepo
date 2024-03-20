Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
  $ ${TURBO} run build --daemon --no-daemon
   ERROR  the argument '--[no-]daemon' cannot be used with '--no-daemon'
  
  Usage: turbo(\.exe)? run --\[no-\]daemon (re)
  
  For more information, try '--help'.
  
  [1]
  $ ${TURBO} run build --since main
   ERROR  the following required arguments were not provided:
    --scope <SCOPE>
  
  Usage: turbo(\.exe)? run --scope <SCOPE> --since <SINCE> (re)
  
  For more information, try '--help'.
  
  [1]
  $ ${TURBO} run build --ignore 'app/**'
   ERROR  the following required arguments were not provided:
    <--filter <FILTER>|--scope <SCOPE>>
  
  Usage: turbo(\.exe)? run --ignore <IGNORE> <--filter <FILTER>|--scope <SCOPE>> (re)
  
  For more information, try '--help'.
  
  [1]
  $ ${TURBO} run build --no-deps
   ERROR  the following required arguments were not provided:
    --scope <SCOPE>
  
  Usage: turbo(\.exe)? run --scope <SCOPE> --no-deps (re)
  
  For more information, try '--help'.
  
  [1]
  $ ${TURBO} run build --include-dependencies
   ERROR  the following required arguments were not provided:
    --scope <SCOPE>
  
  Usage: turbo(\.exe)? run --scope <SCOPE> --include-dependencies (re)
  
  For more information, try '--help'.
  
  [1]
