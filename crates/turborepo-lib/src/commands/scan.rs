use std::{sync::LazyLock, time::Duration};

use console::{style, Style};
use futures::StreamExt;
use tokio_stream::StreamMap;
use turborepo_ui::*;

use super::CommandBase;
use crate::{diagnostics::*, DaemonPaths};

// can't use LazyCell since DialoguerTheme isn't Sync
static DIALOGUER_THEME: LazyLock<DialoguerTheme> = LazyLock::new(|| DialoguerTheme {
    prompt_prefix: style(">>>".to_string()).bright().for_stderr(),
    active_item_prefix: style("  ❯".to_string()).for_stderr().green(),
    inactive_item_prefix: style("   ".to_string()).for_stderr(),
    success_prefix: style("   ".to_string()).for_stderr(),
    prompt_style: Style::new().for_stderr(),
    ..Default::default()
});

/// diagnostics run in parallel however to prevent messages from appearing too
/// quickly we introduce a minimum delay between each message
const INTER_MESSAGE_DELAY: Duration = Duration::from_millis(30);

/// Start a diagnostic session. This command will run a series of diagnostics to
/// help you identify potential performance bottlenecks in your monorepo.
///
/// Note: all lints happen in parallel. For the purposes of displaying output,
///       we demultiplex the output and display it in a single stream, meaning
///       to the user, it looks like the lints are running serially.
pub async fn run(base: CommandBase) -> bool {
    let paths = DaemonPaths::from_repo_root(&base.repo_root);
    let color_config = base.color_config;

    println!("\n{}\n", color_config.rainbow(">>> TURBO SCAN"));
    println!(
        "Turborepo does a lot of work behind the scenes to make your monorepo fast,
however, there are some things you can do to make it even faster. {}\n",
        color!(color_config, BOLD_GREEN, "Let's go!")
    );

    let mut all_events = StreamMap::new();

    let d1 = Box::new(DaemonDiagnostic(paths.clone()));
    let d2 = Box::new(LSPDiagnostic(paths));
    let d3 = Box::new(GitDaemonDiagnostic);
    let d5 = Box::new(UpdateDiagnostic(base.repo_root.clone()));
    let d4 = Box::new(RemoteCacheDiagnostic::new(base));

    let diags: Vec<Box<dyn Diagnostic>> = vec![d1, d2, d3, d4, d5];
    let num_tasks: usize = diags.len();
    for diag in diags {
        let name = diag.name();
        let (tx, rx) = DiagnosticChannel::new();
        diag.execute(tx);
        let wrapper = tokio_stream::wrappers::ReceiverStream::new(rx);
        all_events.insert(name, wrapper);
    }

    let mut complete = 0;
    let mut failed = 0;
    let mut not_applicable = 0;

    while let Some((diag, message)) = all_events.next().await {
        use DiagnosticMessage::*;

        let mut diag_events = all_events.remove(diag).expect("stream not found in map");

        // the allowed opening message is 'started'
        let human_name = match message {
            Started(human_name) => human_name,
            _other => {
                panic!("this is a programming error, please report an issue");
            }
        };

        let bar = start_spinner(&human_name);

        while let Some(message) = diag_events.next().await {
            match message {
                Started(_) => {} // ignore duplicate start events
                LogLine(line) => {
                    bar.println(color!(color_config, GREY, "    {}", line).to_string());
                }
                Request(prompt, mut options, chan) => {
                    let opt = bar.suspend(|| {
                        dialoguer::Select::with_theme(&*DIALOGUER_THEME)
                            .with_prompt(prompt)
                            .items(&options)
                            .default(0)
                            .interact()
                            .unwrap()
                    });

                    chan.send(options.swap_remove(opt)).unwrap();
                }
                Suspend(stopped, resume) => {
                    let bar = bar.clone();
                    let handle = tokio::task::spawn_blocking(move || {
                        bar.suspend(|| {
                            // sender is dropped, so we can unsuspend
                            resume.blocking_recv().ok();
                        });
                    });
                    stopped.send(()).ok(); // suspender doesn't need to be notified so failing ok
                    handle.await.expect("panic in suspend task");
                }
                Done(message) => {
                    bar.finish_with_message(
                        color!(color_config, BOLD_GREEN, "{}", message).to_string(),
                    );
                    complete += 1;
                }
                Failed(message) => {
                    bar.finish_with_message(
                        color!(color_config, BOLD_RED, "{}", message).to_string(),
                    );
                    failed += 1;
                }
                NotApplicable(name) => {
                    let n_a = color!(color_config, GREY, "n/a").to_string();
                    let style = bar.style().tick_strings(&[&n_a, &n_a]);
                    bar.set_style(style);
                    bar.finish_with_message(
                        color!(color_config, BOLD_GREY, "{}", name).to_string(),
                    );
                    not_applicable += 1;
                }
            };
            if complete + not_applicable + failed == num_tasks {
                break;
            }
            tokio::time::sleep(INTER_MESSAGE_DELAY).await;
        }
    }

    if complete + not_applicable == num_tasks {
        println!("\n\n{}", color_config.rainbow(">>> FULL TURBO"));
        true
    } else {
        false
    }
}
