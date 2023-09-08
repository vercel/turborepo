use std::{
    io::{self, BufRead, BufReader, Write},
    process::{Command, Stdio},
};

use console::Style;
use turbopath::AbsoluteSystemPath;
use turborepo_ui::{
    LogWriter, OutputClient, OutputClientBehavior, OutputSink, PrefixedUI, PrefixedWriter, UI,
};

#[test]
fn test_can_write_from_threads() {
    // Construct our output sink and clients
    let mut out = Vec::new();
    let mut err = Vec::new();
    let sink = OutputSink::new(&mut out, &mut err);

    let task1_client = sink.logger(OutputClientBehavior::Grouped);
    let task2_client = sink.logger(OutputClientBehavior::Grouped);

    // Setup the log files used by the tasks
    let log_dir = tempfile::tempdir().unwrap();
    let abs_log_dir = AbsoluteSystemPath::from_std_path(log_dir.path()).unwrap();
    let task1_logfile = abs_log_dir.join_component("task1.txt");
    let task2_logfile = abs_log_dir.join_component("task2.txt");

    // Spawn two tasks that will produce output
    std::thread::scope(|s| {
        let task1_logfile = &task1_logfile;
        let task2_logfile = &task2_logfile;
        s.spawn(move || echo_task("foo", task1_client, task1_logfile));
        s.spawn(move || echo_task("bar", task2_client, task2_logfile));
    });

    assert!(err.is_empty(), "nothing wrote to stderr");
    assert_eq!(
        String::from_utf8(task1_logfile.read().unwrap())
            .unwrap()
            .trim(),
        "hello from foo"
    );
    assert_eq!(
        String::from_utf8(task2_logfile.read().unwrap())
            .unwrap()
            .trim(),
        "hello from bar"
    );

    let output = String::from_utf8(out).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 4, "the two tasks should output two lines each");

    let first_task = lines[0].split(' ').next().unwrap();
    assert_eq!(lines[0], format!("{first_task} > running {first_task}"));
    assert_eq!(lines[1], format!("{first_task} > hello from {first_task}"));
    let second_task = lines[2].split(' ').next().unwrap();
    assert_eq!(lines[2], format!("{second_task} > running {second_task}"));
    assert_eq!(
        lines[3],
        format!("{second_task} > hello from {second_task}")
    );
}

fn echo_task(
    task_name: &'static str,
    client: OutputClient<impl Write>,
    log_file: &AbsoluteSystemPath,
) -> io::Result<()> {
    // Construct the prefix UI used by turbo to write output
    // this output will not appear in a task's log file.
    let output_prefix = Style::new().apply_to(format!("{task_name} > "));
    let warn_prefix = Style::new().apply_to(format!("{task_name} warning > "));
    let ui = UI::new(true);
    let mut prefix_ui = PrefixedUI::new(ui, client.stdout(), client.stderr())
        .with_output_prefix(output_prefix.clone())
        .with_warn_prefix(warn_prefix);

    prefix_ui.output(format!("running {task_name}"));

    // Construct the task logger that will write to the output sink as well as the
    // log file
    let mut task_logger = LogWriter::default();
    task_logger.with_log_file(log_file).unwrap();
    task_logger.with_prefixed_writer(PrefixedWriter::new(ui, output_prefix, client.stdout()));

    let mut cmd = Command::new("echo");
    cmd.args(["hello", "from", task_name]);
    cmd.stdout(Stdio::piped());

    let mut process = cmd.spawn().unwrap();
    let stdout = process.stdout.take().unwrap();

    // Read the process output and send it to the task logger
    let mut buf = String::new();
    let mut reader = BufReader::new(stdout);
    while let Ok(n) = reader.read_line(&mut buf) {
        if n == 0 {
            break;
        } else {
            write!(task_logger, "{buf}").unwrap();
        }
        buf.clear();
    }
    process.wait()?;

    client.finish()?;

    Ok(())
}
