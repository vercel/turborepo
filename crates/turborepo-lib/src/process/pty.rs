use std::io::BufRead;

// Sequence produced by the receiving side of a PTY when receiving EOT
// ^D and then 2 backspaces to overwrite the EOT representation
pub const EOT_SEQUENCE: &[u8] = "^D\u{8}\u{8}".as_bytes();

/// Wrapper for stripping out a single EOT sequence that might appear in a
/// reader
pub struct EotStripper<R> {
    reader: R,
    strip_eot: bool,
}

impl<R: BufRead> EotStripper<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            strip_eot: true,
        }
    }

    pub fn read_line(&mut self, buffer: &mut Vec<u8>) -> std::io::Result<usize> {
        let bytes = self.reader.read_until(b'\n', buffer)?;
        if self.strip_eot {
            // We only check the prefix as EOT is only respected after a newline
            // and portable_pty automatically sends a \n before sending EOT.
            if let Some(trimmed) = buffer.strip_prefix(EOT_SEQUENCE) {
                self.strip_eot = false;
                // Line is just EOT, skip emitting it and move onto next line
                if trimmed.iter().all(|byte| *byte == b'\r' || *byte == b'\n') {
                    buffer.clear();
                    return self.read_line(buffer);
                } else {
                    let mut new_buffer = Vec::with_capacity(trimmed.len());
                    new_buffer.extend_from_slice(trimmed);
                    std::mem::swap(buffer, &mut new_buffer);
                }
            }
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;

    use pretty_assertions::assert_eq;
    use test_case::test_case;

    use super::*;

    #[test_case("no eot here\n", "no eot here\n" ; "no eot")]
    #[test_case("^D\u{8}\u{8}", "" ; "just eot")]
    #[test_case("^D\u{8}\u{8}\r\n", "" ; "just eot + whitespace")]
    #[test_case("^D\u{8}\u{8}message\n", "message\n" ; "eot with output")]
    #[test_case("^D\u{8}\u{8}\r\nmessage\n", "message\n" ; "eot with output on newline")]
    #[test_case("^D\u{8}\u{8}\r\n^D\u{8}\u{8}\r\n", "^D\u{8}\u{8}\r\n" ; "double eot")]
    fn test_eot_stripper(input: &str, expected: &str) {
        let mut stripper = EotStripper::new(input.as_bytes());
        let mut output = Vec::new();
        let mut buffer = Vec::new();
        while stripper.read_line(&mut buffer).unwrap() != 0 {
            output.write_all(&buffer).unwrap()
        }
        let actual = String::from_utf8(output).unwrap();
        assert_eq!(actual, expected)
    }
}
