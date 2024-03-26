mod heaptrack;
mod nextjs;
mod turbopack;

use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    mem::take,
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::Result;

use crate::{
    reader::{heaptrack::HeaptrackFormat, nextjs::NextJsFormat, turbopack::TurbopackFormat},
    store_container::StoreContainer,
};

trait TraceFormat {
    fn read(&mut self, buffer: &[u8]) -> Result<usize>;
}

#[derive(Default)]
enum TraceFile {
    Raw(File),
    Zstd(zstd::Decoder<'static, BufReader<File>>),
    #[default]
    Unloaded,
}

impl TraceFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Raw(file) => file.read(buffer),
            Self::Zstd(decoder) => decoder.read(buffer),
            Self::Unloaded => unreachable!(),
        }
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        match self {
            Self::Raw(file) => file.stream_position(),
            Self::Zstd(decoder) => decoder.get_mut().stream_position(),
            Self::Unloaded => unreachable!(),
        }
    }

    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Raw(file) => file.seek(pos),
            Self::Zstd(decoder) => decoder.get_mut().seek(pos),
            Self::Unloaded => unreachable!(),
        }
    }
}

pub struct TraceReader {
    store: Arc<StoreContainer>,
    path: PathBuf,
}

impl TraceReader {
    pub fn spawn(store: Arc<StoreContainer>, path: PathBuf) -> JoinHandle<()> {
        let mut reader = Self { store, path };
        std::thread::spawn(move || reader.run())
    }

    pub fn run(&mut self) {
        loop {
            self.try_read();
            thread::sleep(Duration::from_millis(500));
        }
    }

    fn trace_file_from_file(&self, file: File) -> io::Result<TraceFile> {
        Ok(if self.path.to_string_lossy().ends_with(".zst") {
            TraceFile::Zstd(zstd::Decoder::new(file)?)
        } else {
            TraceFile::Raw(file)
        })
    }

    fn try_read(&mut self) -> bool {
        let Ok(mut file) = File::open(&self.path) else {
            return false;
        };
        println!("Trace file opened");

        {
            let mut store = self.store.write();
            store.reset();
        }

        let mut format: Option<Box<dyn TraceFormat>> = None;

        let mut initial_read = {
            if let Ok(pos) = file.seek(SeekFrom::End(0)) {
                if pos > 100 * 1024 * 1024 {
                    Some((0, pos))
                } else {
                    None
                }
            } else {
                None
            }
        };
        if file.seek(SeekFrom::Start(0)).is_err() {
            return false;
        }
        let mut file = match self.trace_file_from_file(file) {
            Ok(f) => f,
            Err(err) => {
                println!("Error creating zstd decoder: {err}");
                return false;
            }
        };

        let mut buffer = Vec::new();
        let mut index = 0;

        let mut chunk = vec![0; 8 * 1024 * 1024];
        loop {
            match file.read(&mut chunk) {
                Ok(bytes_read) => {
                    if bytes_read == 0 {
                        if let Some(value) = self.wait_for_more_data(&mut file, &mut initial_read) {
                            return value;
                        }
                    } else {
                        // If we have partially consumed some data, and we are at buffer capacity,
                        // remove the consumed data to make more space.
                        if index > 0 && buffer.len() + bytes_read > buffer.capacity() {
                            buffer.splice(..index, std::iter::empty());
                            index = 0;
                        }
                        buffer.extend_from_slice(&chunk[..bytes_read]);
                        if format.is_none() && buffer.len() >= 8 {
                            if buffer.starts_with(b"TRACEv0") {
                                index = 7;
                                format = Some(Box::new(TurbopackFormat::new(self.store.clone())));
                            } else if buffer.starts_with(b"[{\"name\"") {
                                format = Some(Box::new(NextJsFormat::new(self.store.clone())));
                            } else if buffer.starts_with(b"v ") {
                                format = Some(Box::new(HeaptrackFormat::new(self.store.clone())))
                            } else {
                                // Fallback to the format without magic bytes
                                // TODO Remove this after a while and show an error instead
                                format = Some(Box::new(TurbopackFormat::new(self.store.clone())));
                            }
                        }
                        if let Some(format) = &mut format {
                            match format.read(&buffer[index..]) {
                                Ok(bytes_read) => {
                                    index += bytes_read;
                                }
                                Err(err) => {
                                    println!("Trace file error: {err}");
                                    return true;
                                }
                            }
                            if self.store.want_to_read() {
                                thread::yield_now();
                            }
                            if let Some((current, total)) = &mut initial_read {
                                let old_mbs = *current / (97 * 1024 * 1024);
                                *current += bytes_read as u64;
                                *total = *total.max(current);
                                let new_mbs = *current / (97 * 1024 * 1024);
                                if old_mbs != new_mbs {
                                    println!(
                                        "{}% read ({}/{} MB)",
                                        *current * 100 / *total,
                                        *current / (1024 * 1024),
                                        *total / (1024 * 1024),
                                    );
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    if err.kind() == io::ErrorKind::UnexpectedEof {
                        if let Some(value) = self.wait_for_more_data(&mut file, &mut initial_read) {
                            return value;
                        }
                    } else {
                        // Error reading file, maybe it was removed
                        println!("Error reading trace file: {err:?}");
                        return true;
                    }
                }
            }
        }
    }

    fn wait_for_more_data(
        &mut self,
        file: &mut TraceFile,
        initial_read: &mut Option<(u64, u64)>,
    ) -> Option<bool> {
        let Ok(pos) = file.stream_position() else {
            return Some(true);
        };
        drop(take(file));
        if let Some((_, total)) = initial_read.take() {
            println!("Initial read completed ({} MB)", total / (1024 * 1024),);
        }
        thread::sleep(Duration::from_millis(100));
        let Ok(file_again) = File::open(&self.path) else {
            return Some(true);
        };
        *file = match self.trace_file_from_file(file_again) {
            Ok(f) => f,
            Err(err) => {
                println!("Error creating zstd decoder: {err}");
                return Some(false);
            }
        };
        let Ok(end) = file.seek(SeekFrom::End(0)) else {
            return Some(true);
        };
        // No more data to read, sleep for a while to wait for more data
        if end < pos {
            return Some(true);
        } else if end != pos {
            // Seek to the same position. This will fail when the file was
            // truncated.
            let Ok(new_pos) = file.seek(SeekFrom::Start(pos)) else {
                return Some(true);
            };
            if new_pos != pos {
                return Some(true);
            }
        }
        None
    }
}
