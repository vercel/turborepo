use std::io::{self, Write};

const ESC: u8 = 0x1b;

/// A held sequence longer than this isn't a real color code, so it's written
/// as is. Keeps the carried-over bytes small.
const MAX_SGR_LEN: usize = 64;

/// Removes color codes (SGR sequences such as `ESC[31m` or `ESC[38:2:1:2:3m`)
/// from task output when the user disabled color. Other escape sequences,
/// like cursor movement, pass through so progress redraws keep working.
///
/// Output arrives in arbitrary chunks, so a color code split across writes is
/// held until the write that completes it. A held prefix still unfinished when
/// the writer drops is written unchanged.
pub struct ColorStripWriter<W: Write> {
    inner: W,
    strip: bool,
    pending: Vec<u8>,
}

impl<W: Write> ColorStripWriter<W> {
    pub fn new(inner: W, strip: bool) -> Self {
        Self {
            inner,
            strip,
            pending: Vec::new(),
        }
    }
}

enum Sgr {
    /// A complete color code of this many bytes.
    Complete(usize),
    /// Could still become a color code once more bytes arrive.
    Incomplete,
    /// Some other escape sequence, written unchanged.
    Other,
}

/// `bytes` starts with `ESC`.
fn parse_sgr(bytes: &[u8]) -> Sgr {
    match bytes.get(1) {
        None => return Sgr::Incomplete,
        Some(b'[') => {}
        Some(_) => return Sgr::Other,
    }
    for (index, byte) in bytes.iter().enumerate().skip(2) {
        match byte {
            b'0'..=b'9' | b';' | b':' if index < MAX_SGR_LEN => {}
            b'm' => return Sgr::Complete(index + 1),
            _ => return Sgr::Other,
        }
    }
    Sgr::Incomplete
}

impl<W: Write> Write for ColorStripWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.strip {
            return self.inner.write(buf);
        }

        let joined;
        let input = if self.pending.is_empty() {
            buf
        } else {
            joined = [std::mem::take(&mut self.pending).as_slice(), buf].concat();
            &joined
        };

        let mut output = Vec::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.iter().position(|&byte| byte == ESC) {
            output.extend_from_slice(&rest[..start]);
            rest = &rest[start..];
            match parse_sgr(rest) {
                Sgr::Complete(len) => rest = &rest[len..],
                Sgr::Incomplete => {
                    self.pending = rest.to_vec();
                    rest = &[];
                }
                Sgr::Other => {
                    output.push(ESC);
                    rest = &rest[1..];
                }
            }
        }
        output.extend_from_slice(rest);

        self.inner.write_all(&output)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Write> Drop for ColorStripWriter<W> {
    fn drop(&mut self) {
        if !self.pending.is_empty() {
            let _ = self.inner.write_all(&self.pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::ColorStripWriter;

    fn strip_chunks(chunks: &[&[u8]]) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut writer = ColorStripWriter::new(&mut output, true);
            for chunk in chunks {
                writer.write_all(chunk).unwrap();
            }
        }
        output
    }

    #[test]
    fn strips_color_codes() {
        assert_eq!(
            strip_chunks(&[b"color=\x1b[31mred\x1b[39m \x1b[1;38;2;1;2;3mx\x1b[0m\n"]),
            b"color=red x\n"
        );
        assert_eq!(strip_chunks(&[b"\x1b[38:2::255:0:0mrgb\x1b[m"]), b"rgb");
    }

    #[test]
    fn keeps_other_escape_sequences() {
        let input: &[u8] = b"50%\x1b[1A\x1b[2K\x1b[?25l\x1b]0;title\x07\x1b(B100%";
        assert_eq!(strip_chunks(&[input]), input);
    }

    #[test]
    fn strips_codes_split_across_writes() {
        let input = b"a\x1b[31mred\x1b[39mb";
        for split in 0..=input.len() {
            let (head, tail) = input.split_at(split);
            assert_eq!(strip_chunks(&[head, tail]), b"aredb", "split at {split}");
        }
    }

    #[test]
    fn writes_unfinished_sequence_on_drop() {
        assert_eq!(strip_chunks(&[b"done\x1b[3"]), b"done\x1b[3");
    }

    #[test]
    fn passes_bytes_through_when_disabled() {
        let input: &[u8] = b"\x1b[31mred\x1b[39m\xff";
        let mut output = Vec::new();
        ColorStripWriter::new(&mut output, false)
            .write_all(input)
            .unwrap();
        assert_eq!(output, input);
    }
}
