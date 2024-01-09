use std::{debug_assert, io::Write, thread::JoinHandle};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};

#[derive(Clone, Debug)]
pub struct TraceWriter {
    channel: Sender<Vec<u8>>,
    back_channel: Receiver<Vec<u8>>,
}

impl TraceWriter {
    /// This is a non-blocking writer that writes a file in a background thread.
    /// This is inspired by tracing-appender non_blocking, but has some
    /// differences:
    /// * It allows writing an owned Vec<u8> instead of a reference, so avoiding
    ///   additional allocation.
    /// * It uses an unbounded channel to avoid slowing down the application at
    ///   all (memory) cost.
    /// * It issues less writes by buffering the data into chunks of ~1MB, when
    ///   possible.
    pub fn new<W: Write + Send + 'static>(mut writer: W) -> (Self, TraceWriterGuard) {
        let (tx, rx) = unbounded::<Vec<u8>>();
        let (back_tx, back_rx) = bounded::<Vec<u8>>(1024 * 10);

        let handle: std::thread::JoinHandle<()> = std::thread::spawn(move || {
            let mut buf = Vec::with_capacity(1024 * 1024);
            'outer: loop {
                if !buf.is_empty() {
                    let _ = writer.write_all(&buf);
                    let _ = writer.flush();
                    buf.clear();
                }
                let Ok(mut data) = rx.recv() else {
                    break 'outer;
                };
                if data.is_empty() {
                    break 'outer;
                }
                if data.len() > buf.capacity() {
                    let _ = writer.write_all(&data);
                } else {
                    buf.extend_from_slice(&data);
                }
                data.clear();
                let _ = back_tx.try_send(data);
                loop {
                    match rx.try_recv() {
                        Ok(data) => {
                            if data.is_empty() {
                                break 'outer;
                            }
                            if buf.len() + data.len() > buf.capacity() {
                                let _ = writer.write_all(&buf);
                                buf.clear();
                                if data.len() > buf.capacity() {
                                    let _ = writer.write_all(&data);
                                } else {
                                    buf.extend_from_slice(&data);
                                }
                            } else {
                                buf.extend_from_slice(&data);
                            }
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
            if !buf.is_empty() {
                let _ = writer.write_all(&buf);
            }
            let _ = writer.flush();
            drop(writer);
        });

        (
            Self {
                channel: tx.clone(),
                back_channel: back_rx.clone(),
            },
            TraceWriterGuard {
                channel: Some(tx),
                back_channel: Some(back_rx),
                handle: Some(handle),
            },
        )
    }

    pub fn write(&self, data: Vec<u8>) {
        debug_assert!(!data.is_empty());
        let _ = self.channel.send(data);
    }

    pub fn try_get_buffer(&self) -> Option<Vec<u8>> {
        self.back_channel.try_recv().ok()
    }
}

pub struct TraceWriterGuard {
    channel: Option<Sender<Vec<u8>>>,
    back_channel: Option<Receiver<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for TraceWriterGuard {
    fn drop(&mut self) {
        let _ = self.channel.take().unwrap().send(Vec::new());
        let back_channel = self.back_channel.take().unwrap();
        while back_channel.recv().is_ok() {}
        let _ = self.handle.take().unwrap().join();
    }
}
