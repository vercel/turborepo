use serde::{Deserialize, Serialize};
use turbo_tasks::trace::TraceRawVcs;
use turbo_tasks_hash::DeterministicHash;

/// LINE FEED (LF), one of the basic JS line terminators.
const U8_LF: u8 = 0x0A;
/// CARRIAGE RETURN (CR), one of the basic JS line terminators.
const U8_CR: u8 = 0x0D;

#[derive(
    Default,
    Debug,
    PartialEq,
    Eq,
    Copy,
    Clone,
    PartialOrd,
    Ord,
    TraceRawVcs,
    Serialize,
    Deserialize,
    DeterministicHash,
)]
pub struct SourcePos {
    pub line: usize,
    pub column: usize,
}

impl SourcePos {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn max() -> Self {
        Self {
            line: usize::MAX,
            column: usize::MAX,
        }
    }

    /// Increments the line/column position to account for new source code.
    /// Line terminators are the classic "\n", "\r", "\r\n" (which counts as
    /// a single terminator), and JSON LINE/PARAGRAPH SEPARATORs.
    ///
    /// See https://tc39.es/ecma262/multipage/ecmascript-language-lexical-grammar.html#sec-line-terminators
    pub fn update(&mut self, code: &[u8]) {
        // JS source text is interpreted as UCS-2, which is basically UTF-16 with less
        // restrictions. We cannot iterate UTF-8 bytes here, 2-byte UTF-8 octets
        // should count as a 1 char and not 2.
        let SourcePos {
            mut line,
            mut column,
        } = self;
        let mut i = 0;
        while i < code.len() {
            match code[i] {
                U8_LF => {
                    i += 1;
                    line += 1;
                    column = 0;
                }
                U8_CR => {
                    // Count "\r\n" as a single terminator.
                    if code.get(i + 1) == Some(&U8_LF) {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    line += 1;
                    column = 0;
                }
                b if b & 0b10000000 == 0 => {
                    i += 1;
                    column += 1;
                }
                b if b & 0b11100000 == 0b11000000 => {
                    // eat 1 octet
                    i += 2;
                    column += 1;
                }
                b if b & 0b11110000 == 0b11100000 => {
                    let mut separator = false;
                    if code.get(i + 1) == Some(&0b10000000) {
                        if let Some(b) = code.get(i + 2) {
                            separator = (b & 0b11111110) == 0b10101000
                        }
                    }
                    // eat 2 octets
                    i += 3;
                    if separator {
                        line += 1;
                        column = 0;
                    } else {
                        column += 1;
                    }
                }
                _ => {
                    // eat 3 octets
                    i += 4;
                    // Surrogate pair
                    column += 2;
                }
            }
        }
        self.line = line;
        self.column = column;
    }
}

impl std::cmp::PartialEq<(usize, usize)> for SourcePos {
    fn eq(&self, other: &(usize, usize)) -> bool {
        &(self.line, self.column) == other
    }
}
