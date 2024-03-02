Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/../../helpers/mock_telemetry_config.sh

Run status (with first run message)
  $ TURBO_TELEMETRY_MESSAGE_DISABLED=0 ${TURBO} telemetry status
  
  Attention:
  Turborepo now collects completely anonymous telemetry regarding usage.
  This information is used to shape the Turborepo roadmap and prioritize features.
  You can learn more, including how to opt-out if you'd not like to participate in this anonymous program, by visiting the following URL:
  https://turbo.build/repo/docs/telemetry
  
  
  Status: Enabled
  
  Turborepo telemetry is completely anonymous. Thank you for participating!
  Learn more: https://turbo.build/repo/docs/telemetry

Run without command
  $ ${TURBO} telemetry
  
  Status: Enabled
  
  Turborepo telemetry is completely anonymous. Thank you for participating!
  Learn more: https://turbo.build/repo/docs/telemetry

Disable
  $ ${TURBO} telemetry disable
  Success!
  
  Status: Disabled
  
  You have opted-out of Turborepo anonymous telemetry. No data will be collected from your machine.
  Learn more: https://turbo.build/repo/docs/telemetry

Enable
  $ ${TURBO} telemetry enable
  Success!
  
  Status: Enabled
  
  Turborepo telemetry is completely anonymous. Thank you for participating!
  Learn more: https://turbo.build/repo/docs/telemetry


