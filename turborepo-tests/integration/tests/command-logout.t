Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/logged_in.sh

Logout while logged in
  $ ${TURBO} logout
  
  Attention:
  Turborepo now collects completely anonymous telemetry regarding usage.
  This information is used to shape the Turborepo roadmap and prioritize features.
  You can learn more, including how to opt-out if you'd not like to participate in this anonymous program, by visiting the following URL:
  https://turbo.build/repo/docs/telemetry
  
  >>> Logged out

Logout while logged out
  $ ${TURBO} logout
  >>> Logged out

