use std::{error::Error, io, sync::mpsc, time::Duration};

use crossterm::{
    event::KeyCode,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    text::Text,
    widgets::{Paragraph, Widget},
    Terminal, TerminalOptions, Viewport,
};
use turborepo_ui::TerminalPane;

fn main() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(24),
        },
    )?;

    terminal.insert_before(1, |buf| {
        Text::raw("Press q to exit, use arrow keys to switch panes").render(buf.area, buf)
    })?;

    let (tx, rx) = mpsc::sync_channel(1);

    std::thread::spawn(move || handle_input(tx));

    let size = terminal.get_frame().size();

    let pane = TerminalPane::new(
        size.height,
        size.width,
        vec![
            ("foo".into(), None),
            ("bar".into(), None),
            ("baz".into(), None),
        ],
    );

    run_app(&mut terminal, pane, rx)?;

    terminal.clear()?;

    // restore terminal
    disable_raw_mode()?;
    terminal.show_cursor()?;
    println!();

    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut pane: TerminalPane<()>,
    rx: mpsc::Receiver<Event>,
) -> io::Result<()> {
    let tasks = ["foo", "bar", "baz"];
    let mut idx: usize = 0;
    pane.select("foo").unwrap();
    let mut tick = 0;
    while let Ok(event) = rx.recv() {
        match event {
            Event::Up => {
                idx = idx.saturating_sub(1);
                let task = tasks[idx];
                pane.select(task).unwrap();
            }
            Event::Down => {
                idx = (idx + 1).clamp(0, 2);
                let task = tasks[idx];
                pane.select(task).unwrap();
            }
            Event::Stop => break,
            Event::Tick => {
                if tick % 3 == 0 {
                    let color = format!("\x1b[{}m", 30 + (tick % 10));
                    for task in tasks {
                        pane.process_output(
                            task,
                            format!("{task}: {color}tick {tick}\x1b[0m\r\n").as_bytes(),
                        )
                        .unwrap();
                    }
                }
            }
        }
        terminal.draw(|f| f.render_widget(&pane, f.size()))?;
        tick += 1;
    }

    Ok(())
}
enum Event {
    Up,
    Down,
    Stop,
    Tick,
}

fn handle_input(tx: mpsc::SyncSender<Event>) -> std::io::Result<()> {
    loop {
        if crossterm::event::poll(Duration::from_millis(20))? {
            let event = crossterm::event::read()?;
            if let crossterm::event::Event::Key(key_event) = event {
                if let Some(event) = match key_event.code {
                    KeyCode::Up => Some(Event::Up),
                    KeyCode::Down => Some(Event::Down),
                    KeyCode::Char('q') => Some(Event::Stop),
                    _ => None,
                } {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            }
        } else if tx.send(Event::Tick).is_err() {
            break;
        }
    }
    Ok(())
}
