pub enum Event {
    StartTask {
        task: String,
    },
    TaskOutput {
        task: String,
        output: Vec<u8>,
    },
    EndTask {
        task: String,
    },
    Stop,
    Tick,
    Log {
        message: Vec<u8>,
    },
    Up,
    Down,
    SetStdin {
        task: String,
        stdin: Box<dyn std::io::Write + Send>,
    },
    EnterInteractive,
    ExitInteractive,
    Input {
        bytes: Vec<u8>,
    },
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn assert_event_send() {
        fn send_sync<T: Send>() {}
        send_sync::<Event>();
    }
}
