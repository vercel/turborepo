use std::io::Write;

/// Writer that will buffer writes so the underlying writer is only called with
/// writes that end in a newline
pub struct LineWriter<W> {
    writer: W,
    buffer: Vec<u8>,
}

impl<W: Write> LineWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            buffer: Vec::with_capacity(512),
        }
    }
}

impl<W: Write> Write for LineWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for line in buf.split_inclusive(|c| *c == b'\n') {
            if line.ends_with(b"\n") {
                if self.buffer.is_empty() {
                    self.writer.write_all(line)?;
                } else {
                    self.buffer.extend_from_slice(line);
                    self.writer.write_all(&self.buffer)?;
                    self.buffer.clear();
                }
            } else {
                // This should only happen on the last chunk?
                self.buffer.extend_from_slice(line)
            }
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // We don't flush our buffer as that would lead to a write without a newline
        self.writer.flush()
    }
}
