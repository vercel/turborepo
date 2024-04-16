Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
  $ ${TURBO} run build --daemon --no-daemon
   ERROR  the argument '--\[no-\]daemon' cannot be used with '--no-daemon' (re)
  
  Usage: turbo(\.exe)? run --\[no-\]daemon (re)
  
  For more information, try '--help'.
  
  [1]

  $ ${TURBO} run build --ignore 'app/**'
   ERROR  the following required arguments were not provided:
    <--filter <FILTER>>
  
  Usage: turbo(\.exe)? run --ignore <IGNORE> <--filter <FILTER>> (re)
  
  For more information, try '--help'.
  
  [1]
