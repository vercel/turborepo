Setup
  $ . ${TESTDIR}/setup.sh

Test help flag
  $ ${TURBO} -h
  Usage: turbo [--version] [--help] <command> [<args>]
  
  Available commands are:
      bin       Get the path to the Turbo binary
      daemon    Runs turbod
      link      Link your local directory to a Vercel organization and enable remote caching.
      login     Login to your Vercel account
      logout    Logout of your Vercel account
      prune     Prepare a subset of your monorepo.
      run       Run tasks across projects in your monorepo
      unlink    Unlink the current directory from your Vercel organization and disable Remote Caching
  

  $ ${TURBO} --help
  Usage: turbo [--version] [--help] <command> [<args>]
  
  Available commands are:
      bin       Get the path to the Turbo binary
      daemon    Runs turbod
      link      Link your local directory to a Vercel organization and enable remote caching.
      login     Login to your Vercel account
      logout    Logout of your Vercel account
      prune     Prepare a subset of your monorepo.
      run       Run tasks across projects in your monorepo
      unlink    Unlink the current directory from your Vercel organization and disable Remote Caching
  
