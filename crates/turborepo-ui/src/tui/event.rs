#[derive(Debug, Clone)]
pub enum Event {
    StartTask { task: String },
    TaskOutput { task: String, output: Vec<u8> },
    EndTask { task: String },
    Stop,
    Tick,
    Log { message: Vec<u8> },
    Up,
    Down,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn assert_event_sync_send() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Event>();
    }
}
