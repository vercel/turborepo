use std::time::Instant;

enum Event {
    Tick,
}

struct State {
    current_time: Instant,
}

struct Focus {
    task_id: String,
    focus_type: FocusType,
}

enum FocusType {
    View,
    Interact,
}

struct DoneTask {
    id: String,
    start: Instant,
    end: Instant,
}
