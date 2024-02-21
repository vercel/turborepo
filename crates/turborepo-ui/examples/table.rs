use std::{error::Error, io, sync::mpsc, time::Duration};

use crossterm::{
    event::{KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use turborepo_ui::TaskTable;

enum Event {
    Tick(u64),
    Start(&'static str),
    Finish(&'static str),
    Up,
    Down,
    Stop,
}

fn main() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(8),
        },
    )?;

    let (tx, rx) = mpsc::sync_channel(1);
    let input_tx = tx.clone();
    // Thread forwards user input
    let input = std::thread::spawn(move || handle_input(input_tx));
    // Thread simulates starting/finishing of tasks
    let events = std::thread::spawn(move || send_events(tx));

    let table = TaskTable::new((0..6).map(|i| format!("task_{i}")));

    run_app(&mut terminal, table, rx)?;

    events.join().expect("event thread panicked");
    input.join().expect("input thread panicked")?;

    // restore terminal
    disable_raw_mode()?;
    terminal.show_cursor()?;
    println!();

    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut table: TaskTable,
    rx: mpsc::Receiver<Event>,
) -> io::Result<()> {
    while let Ok(event) = rx.recv() {
        match event {
            Event::Tick(_) => {
                table.tick();
            }
            Event::Start(task) => table.start_task(task).unwrap(),
            Event::Finish(task) => table.finish_task(task).unwrap(),
            Event::Up => table.previous(),
            Event::Down => table.next(),
            Event::Stop => break,
        }
        terminal.draw(|f| table.stateful_render(f))?;
    }

    Ok(())
}

fn send_events(tx: mpsc::SyncSender<Event>) {
    let mut events = vec![
        Event::Start("task_0"),
        Event::Start("task_1"),
        Event::Tick(10),
        Event::Start("task_2"),
        Event::Tick(30),
        Event::Start("task_3"),
        Event::Finish("task_2"),
        Event::Tick(30),
        Event::Start("task_4"),
        Event::Finish("task_0"),
        Event::Tick(10),
        Event::Finish("task_1"),
        Event::Start("task_5"),
        Event::Tick(30),
        Event::Finish("task_3"),
        Event::Finish("task_4"),
        Event::Tick(50),
        Event::Finish("task_5"),
        Event::Stop,
    ];
    events.reverse();
    while let Some(event) = events.pop() {
        if let Event::Tick(ticks) = event {
            std::thread::sleep(Duration::from_millis(50 * ticks));
        }
        if tx.send(event).is_err() {
            break;
        }
    }
}

fn handle_input(tx: mpsc::SyncSender<Event>) -> std::io::Result<()> {
    loop {
        if crossterm::event::poll(Duration::from_millis(10))? {
            let event = crossterm::event::read()?;
            if let crossterm::event::Event::Key(key_event) = event {
                if let Some(event) = match key_event.code {
                    KeyCode::Up => Some(Event::Up),
                    KeyCode::Down => Some(Event::Down),
                    KeyCode::Char('c') if key_event.modifiers == KeyModifiers::CONTROL => {
                        Some(Event::Stop)
                    }
                    _ => None,
                } {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            }
        } else if tx.send(Event::Tick(0)).is_err() {
            break;
        }
    }
    Ok(())
}
