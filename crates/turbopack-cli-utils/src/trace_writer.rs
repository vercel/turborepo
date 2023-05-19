// This is inspired by tracing-appender non_blocking, but allows writing a owned
// Vec<u8> instead of a reference, and uses a unbounded channel to avoid slowing
// down the application.

use std::{debug_assert, io::Write, thread::JoinHandle};

use crossbeam_channel::{unbounded, Sender, TryRecvError};

#[derive(Clone, Debug)]
pub struct TraceWriter {
    channel: Sender<Vec<u8>>,
}

impl TraceWriter {
    pub fn new<W: Write + Send + 'static>(mut writer: W) -> (Self, TraceWriterGuard) {
        let (tx, rx) = unbounded::<Vec<u8>>();

        let handle: std::thread::JoinHandle<()> = std::thread::spawn(move || {
            'outer: loop {
                let Ok(data) = rx.recv() else {
                    break 'outer;
                };
                let _ = writer.write_all(&data);
                loop {
                    match rx.try_recv() {
                        Ok(data) => {
                            if data.is_empty() {
                                break 'outer;
                            }
                            let _ = writer.write_all(&data);
                        }
                        Err(TryRecvError::Disconnected) => {
                            break 'outer;
                        }
                        Err(TryRecvError::Empty) => {
                            break;
                        }
                    }
                }
            }
            let _ = writer.flush();
            drop(writer);
        });

        (
            Self {
                channel: tx.clone(),
            },
            TraceWriterGuard {
                channel: Some(tx),
                handle: Some(handle),
            },
        )
    }

    pub fn write(&self, data: Vec<u8>) {
        debug_assert!(!data.is_empty());
        let _ = self.channel.send(data);
    }
}

pub struct TraceWriterGuard {
    channel: Option<Sender<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for TraceWriterGuard {
    fn drop(&mut self) {
        let _ = self.channel.take().unwrap().send(Vec::new());
        let _ = self.handle.take().unwrap().join();
    }
}
