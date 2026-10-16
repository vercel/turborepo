//! zstd output stage for archives written to the local cache.
//!
//! Compressing an archive with large outputs on one thread is the longest
//! part of a cache write, and it can't start on a chunk until the files in
//! it have been read. [`ZstdFileSink`] writes small archives straight into
//! the encoder. Once an archive has grown past [`PIPELINE_THRESHOLD`], it
//! moves the encoder to a dedicated thread and feeds it 1 MiB chunks, so
//! reading files and building tar entries overlaps with compression.
//!
//! The compressed bytes are identical to writing the same stream into a
//! single encoder. zstd's streaming output depends only on the input bytes,
//! not on how they are split across `write` calls, as long as the encoder
//! is never flushed before it is finished.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread::JoinHandle,
};

/// Size of the pieces handed to the compression thread.
const CHUNK_SIZE: usize = 1 << 20;
/// Uncompressed bytes written inline before moving compression to a thread.
/// Smaller archives take exactly the single-threaded path.
pub(crate) const PIPELINE_THRESHOLD: usize = 4 << 20;
/// Chunks that may wait for the compression thread. Reading is faster than
/// compressing, so a short queue keeps the thread busy while bounding memory.
const PIPELINE_DEPTH: usize = 2;

type Encoder = zstd::Encoder<'static, BufWriter<File>>;

enum Message {
    Start(Encoder),
    Data(Vec<u8>),
    Finish,
}

enum State {
    Inline {
        encoder: Encoder,
        written: usize,
    },
    Threaded {
        staged: Vec<u8>,
        messages: SyncSender<Message>,
        recycled: Receiver<Vec<u8>>,
        worker: JoinHandle<io::Result<()>>,
    },
    Done,
}

/// Compresses a tar stream into a file. See the module documentation.
pub(crate) struct ZstdFileSink {
    state: State,
    threshold: usize,
}

impl ZstdFileSink {
    pub(crate) fn new(encoder: Encoder) -> Self {
        Self::with_threshold(encoder, PIPELINE_THRESHOLD)
    }

    pub(crate) fn with_threshold(encoder: Encoder, threshold: usize) -> Self {
        Self {
            state: State::Inline {
                encoder,
                written: 0,
            },
            threshold,
        }
    }

    /// Compresses everything written so far, ends the zstd frame, and flushes
    /// the file. The archive is complete on disk when this returns `Ok`.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        match std::mem::replace(&mut self.state, State::Done) {
            State::Inline { encoder, .. } => encoder.finish()?.flush(),
            State::Threaded {
                staged,
                messages,
                worker,
                ..
            } => {
                let sent = (staged.is_empty() || messages.send(Message::Data(staged)).is_ok())
                    && messages.send(Message::Finish).is_ok();
                drop(messages);
                join(worker)?;
                if sent {
                    Ok(())
                } else {
                    Err(io::Error::other("cache compression thread stopped early"))
                }
            }
            State::Done => Ok(()),
        }
    }

    /// Hands the full staged chunk to the compression thread.
    fn send_staged(&mut self) -> io::Result<()> {
        let State::Threaded {
            staged,
            messages,
            recycled,
            ..
        } = &mut self.state
        else {
            unreachable!("only a threaded sink stages chunks");
        };
        let next = recycled
            .try_recv()
            .unwrap_or_else(|_| Vec::with_capacity(CHUNK_SIZE));
        let chunk = std::mem::replace(staged, next);
        if messages.send(Message::Data(chunk)).is_ok() {
            return Ok(());
        }
        // The worker only hangs up after a write error.
        let State::Threaded { worker, .. } = std::mem::replace(&mut self.state, State::Done) else {
            unreachable!("state was just matched as threaded");
        };
        join(worker)?;
        Err(io::Error::other("cache compression thread stopped early"))
    }

    /// Moves the encoder to a compression thread. If the thread can't be
    /// spawned, compression stays inline.
    fn start_worker(&mut self) {
        let (messages, receiver) = mpsc::sync_channel(PIPELINE_DEPTH);
        let (recycle, recycled) = mpsc::channel();
        let Ok(worker) = std::thread::Builder::new()
            .name("turbo-cache-compress".to_string())
            .spawn(move || compress(receiver, recycle))
        else {
            return;
        };
        let State::Inline { encoder, .. } = std::mem::replace(&mut self.state, State::Done) else {
            unreachable!("only an inline sink starts a worker");
        };
        // The worker is blocked waiting for this message, so it can't fail.
        let _ = messages.send(Message::Start(encoder));
        self.state = State::Threaded {
            staged: Vec::with_capacity(CHUNK_SIZE),
            messages,
            recycled,
            worker,
        };
    }
}

impl Write for ZstdFileSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.state {
            State::Inline { encoder, written } => {
                let len = encoder.write(buf)?;
                *written += len;
                if *written >= self.threshold {
                    self.start_worker();
                }
                Ok(len)
            }
            State::Threaded { staged, .. } => {
                // Fill the chunk exactly so its buffer never reallocates.
                let len = buf.len().min(CHUNK_SIZE - staged.len());
                staged.extend_from_slice(&buf[..len]);
                if staged.len() == CHUNK_SIZE {
                    self.send_staged()?;
                }
                Ok(len)
            }
            State::Done => Err(io::Error::other("cache archive already finished")),
        }
    }

    /// Does nothing: data is committed by [`ZstdFileSink::finish`]. Flushing
    /// the encoder mid-stream would end a zstd block early and change the
    /// archive's bytes.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for ZstdFileSink {
    fn drop(&mut self) {
        // Abandoned without `finish` (an error path): stop the worker without
        // completing the archive, and wait so it doesn't outlive the writer.
        if let State::Threaded {
            messages, worker, ..
        } = std::mem::replace(&mut self.state, State::Done)
        {
            drop(messages);
            let _ = worker.join();
        }
    }
}

fn compress(messages: Receiver<Message>, recycle: Sender<Vec<u8>>) -> io::Result<()> {
    let Ok(Message::Start(mut encoder)) = messages.recv() else {
        return Ok(());
    };
    loop {
        match messages.recv() {
            Ok(Message::Data(mut chunk)) => {
                encoder.write_all(&chunk)?;
                chunk.clear();
                let _ = recycle.send(chunk);
            }
            Ok(Message::Finish) => return encoder.finish()?.flush(),
            // The writer was dropped without finishing: abandon the archive.
            Ok(Message::Start(_)) | Err(_) => return Ok(()),
        }
    }
}

fn join(worker: JoinHandle<io::Result<()>>) -> io::Result<()> {
    worker
        .join()
        .map_err(|_| io::Error::other("cache compression thread panicked"))?
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use anyhow::Result;

    use super::*;

    /// Deterministic, compressible text.
    fn sample(len: usize) -> Vec<u8> {
        let words = [
            "const ", "return ", "value ", "props", "; ", "\n", "=> ", "items ",
        ];
        let mut state: u32 = 0x2545_f491;
        let mut out = Vec::with_capacity(len + 8);
        while out.len() < len {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.extend_from_slice(words[(state >> 29) as usize].as_bytes());
            if state.is_multiple_of(7) {
                out.extend_from_slice(format!("{state:x}").as_bytes());
            }
        }
        out.truncate(len);
        out
    }

    fn compress_with(data: &[u8], threshold: usize) -> Result<Vec<u8>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("out.tar.zst");
        let file = BufWriter::new(File::create(&path)?);
        let mut sink = ZstdFileSink::with_threshold(zstd::Encoder::new(file, 0)?, threshold);
        // Write in tar-like pieces that don't line up with chunk boundaries.
        for piece in data.chunks(8 * 1024 + 17) {
            sink.write_all(piece)?;
        }
        sink.finish()?;
        Ok(std::fs::read(&path)?)
    }

    #[test]
    fn output_matches_a_single_encoder() -> Result<()> {
        let data = sample(3 * CHUNK_SIZE + 12_345);
        let mut reference = zstd::Encoder::new(Vec::new(), 0)?;
        reference.write_all(&data)?;
        let reference = reference.finish()?;

        // Inline only, inline then threaded, and threaded after the first write.
        for threshold in [usize::MAX, 2 * CHUNK_SIZE, 0] {
            assert!(
                compress_with(&data, threshold)? == reference,
                "output differs with threshold {threshold}"
            );
        }

        let mut decoded = Vec::new();
        zstd::Decoder::new(&reference[..])?.read_to_end(&mut decoded)?;
        assert!(decoded == data);
        Ok(())
    }

    #[test]
    fn small_output_matches_a_single_encoder() -> Result<()> {
        let data = sample(10_000);
        let mut reference = zstd::Encoder::new(Vec::new(), 0)?;
        reference.write_all(&data)?;
        assert!(compress_with(&data, PIPELINE_THRESHOLD)? == reference.finish()?);
        Ok(())
    }

    #[test]
    fn abandoned_sink_stops_its_worker() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = BufWriter::new(File::create(dir.path().join("out.tar.zst"))?);
        let mut sink = ZstdFileSink::with_threshold(zstd::Encoder::new(file, 0)?, 0);
        sink.write_all(&sample(3 * CHUNK_SIZE))?;
        assert!(matches!(sink.state, State::Threaded { .. }));
        // Dropping without `finish` must not hang or panic.
        drop(sink);
        Ok(())
    }
}
