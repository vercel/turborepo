//! CRLF→LF normalization for file hashing.
//!
//! When `.gitattributes` marks a file as `text` or `text=auto`, git normalizes
//! CRLF line endings to LF in blob objects. This module replicates that
//! behavior so that turbo's file hashes match git's, regardless of whether
//! we're using the git code path or the manual (no-git) code path.
//!
//! # Known Limitations
//!
//! - Only the root `.gitattributes` is loaded. Nested per-directory
//!   `.gitattributes` files are not supported.
//! - The `eol=` attribute is not handled; only `text`, `text=auto`, `-text`,
//!   and `binary` are recognized.

use std::io::{Read, Seek, SeekFrom};

use gix_attributes::{Search, search::MetadataCollection};
use sha1::{Digest, Sha1};
use tracing::warn;
use turbopath::AbsoluteSystemPath;

use crate::OidHash;

/// How the `text` attribute is set for a given file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextAttr {
    /// `text` — always normalize CRLF→LF.
    Set,
    /// `text=auto` — normalize if the file content appears to be text.
    Auto,
    /// `-text` or `binary` — never normalize.
    Unset,
    /// Attribute not mentioned — don't normalize.
    Unspecified,
}

/// Loaded `.gitattributes` context for resolving per-file text attributes.
///
/// Only the root-level `.gitattributes` is loaded. Per-directory
/// `.gitattributes` files (e.g. `src/.gitattributes`) are not consulted.
pub(crate) struct GitAttrs {
    search: Search,
    collection: MetadataCollection,
}

/// Matches git's `FIRST_FEW_BYTES` (8KB) used by `buffer_is_binary()` to
/// detect binary content. A file is considered binary if a NUL byte appears
/// in the first 8KB. Do not change without verifying against git's behavior.
const GIT_BINARY_DETECT_LEN: usize = 8 * 1024;
const BUF_SIZE: usize = 64 * 1024;

impl GitAttrs {
    /// Load `.gitattributes` from `root`. Returns `None` if no file exists or
    /// the file cannot be parsed.
    pub(crate) fn load(root: &AbsoluteSystemPath) -> Option<Self> {
        let mut buf = Vec::new();
        let mut collection = MetadataCollection::default();

        // Initialize with built-in macros (e.g. `binary` → `-diff -merge -text`)
        let mut search = match Search::new_globals(
            std::iter::empty::<std::path::PathBuf>(),
            &mut buf,
            &mut collection,
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to initialize gitattributes globals: {e}");
                Search::default()
            }
        };

        let gitattributes_path = root.join_component(".gitattributes");
        let added = match search.add_patterns_file(
            gitattributes_path.as_std_path().into(),
            false,
            Some(root.as_std_path()),
            &mut buf,
            &mut collection,
            true,
        ) {
            Ok(added) => added,
            Err(e) => {
                warn!(
                    path = %gitattributes_path,
                    "failed to parse .gitattributes, CRLF normalization will be skipped: {e}"
                );
                false
            }
        };

        if !added {
            return None;
        }

        Some(Self { search, collection })
    }

    /// Create a reusable `Outcome` for repeated calls to
    /// [`resolve_text_attr_with`]. Callers in hot loops (e.g. rayon
    /// parallel iterators) should create one per thread and pass it into
    /// each call to avoid per-file allocation.
    pub(crate) fn new_outcome(&self) -> gix_attributes::search::Outcome {
        let mut outcome = gix_attributes::search::Outcome::default();
        outcome.initialize(&self.collection);
        outcome
    }

    /// Resolve the `text` attribute using a caller-supplied `Outcome`,
    /// avoiding per-file allocation. The outcome is reset before each
    /// query so it can be reused across calls.
    pub(crate) fn resolve_text_attr_with(
        &self,
        relative_path: &str,
        outcome: &mut gix_attributes::search::Outcome,
    ) -> TextAttr {
        outcome.reset();

        let matched = self.search.pattern_matching_relative_path(
            relative_path.into(),
            gix_attributes::glob::pattern::Case::Sensitive,
            Some(false),
            outcome,
        );

        if !matched {
            return TextAttr::Unspecified;
        }

        Self::text_attr_from_outcome(outcome)
    }

    /// Convenience method that allocates an `Outcome` per call. Prefer
    /// [`resolve_text_attr_with`] in hot paths.
    pub(crate) fn resolve_text_attr(&self, relative_path: &str) -> TextAttr {
        let mut outcome = self.new_outcome();
        self.resolve_text_attr_with(relative_path, &mut outcome)
    }

    fn text_attr_from_outcome(outcome: &gix_attributes::search::Outcome) -> TextAttr {
        for m in outcome.iter() {
            if m.assignment.name.as_str() == "text" {
                return match m.assignment.state {
                    gix_attributes::StateRef::Set => TextAttr::Set,
                    gix_attributes::StateRef::Unset => TextAttr::Unset,
                    gix_attributes::StateRef::Value(v) => {
                        if v.as_bstr() == "auto" {
                            TextAttr::Auto
                        } else {
                            TextAttr::Set
                        }
                    }
                    gix_attributes::StateRef::Unspecified => TextAttr::Unspecified,
                };
            }
        }

        TextAttr::Unspecified
    }
}

/// Resolve cached attrs or load them from disk on demand.
///
/// `storage` provides owned backing when `cached` is `None` — callers
/// declare `let mut storage = None;` and pass `&mut storage` here.
pub(crate) fn resolve_or_load<'a>(
    cached: Option<&'a GitAttrs>,
    root: &AbsoluteSystemPath,
    storage: &'a mut Option<GitAttrs>,
) -> Option<&'a GitAttrs> {
    match cached {
        Some(a) => Some(a),
        None => {
            *storage = GitAttrs::load(root);
            storage.as_ref()
        }
    }
}

/// Whether normalization should actually be applied given the text attribute
/// and the file's scan results. Centralizes the decision so the gix and sha1
/// hash paths cannot diverge.
fn should_normalize(attr: TextAttr, scan: &ScanResult) -> bool {
    match attr {
        TextAttr::Set => scan.crlf_count > 0,
        TextAttr::Auto => !scan.is_binary && scan.crlf_count > 0,
        TextAttr::Unset | TextAttr::Unspecified => false,
    }
}

struct ScanResult {
    /// Total raw byte count (including \r bytes from CRLF pairs).
    byte_count: u64,
    /// Number of \r\n pairs found.
    crlf_count: u64,
    /// Whether a NUL byte was found in the first 8KB (only meaningful when
    /// `detect_binary` was true).
    is_binary: bool,
}

/// Scan a file to count CRLF pairs, total byte length, and optionally detect
/// binary content (NUL in first 8KB).
#[cfg(test)]
fn scan_file(reader: &mut impl Read, detect_binary: bool) -> std::io::Result<ScanResult> {
    scan_file_and_feed(reader, detect_binary, |_| {})
}

/// Same as [`scan_file`], but also feeds each raw chunk to `on_data`. This
/// lets callers fuse the scan pass with a hash computation so the common case
/// (no normalization needed) completes in a single read of the file.
fn scan_file_and_feed(
    reader: &mut impl Read,
    detect_binary: bool,
    mut on_data: impl FnMut(&[u8]),
) -> std::io::Result<ScanResult> {
    let mut byte_count: u64 = 0;
    let mut crlf_count: u64 = 0;
    let mut is_binary = false;
    let mut prev_was_cr = false;
    let mut buf = [0u8; BUF_SIZE];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }

        if detect_binary && !is_binary && byte_count < GIT_BINARY_DETECT_LEN as u64 {
            let check_end = n.min(GIT_BINARY_DETECT_LEN.saturating_sub(byte_count as usize));
            if buf[..check_end].contains(&0) {
                is_binary = true;
            }
        }

        for &b in &buf[..n] {
            if b == b'\n' && prev_was_cr {
                crlf_count += 1;
            }
            prev_was_cr = b == b'\r';
        }

        on_data(&buf[..n]);
        byte_count += n as u64;
    }

    Ok(ScanResult {
        byte_count,
        crlf_count,
        is_binary,
    })
}

/// Stream file contents through `update`, normalizing CRLF→LF.
///
/// The output buffer is allocated once and reused across chunks, bounding
/// memory at ~128KB per call (64KB read buffer + 64KB output buffer).
fn stream_normalized(reader: &mut impl Read, mut update: impl FnMut(&[u8])) -> std::io::Result<()> {
    let mut buf = [0u8; BUF_SIZE];
    let mut out = Vec::with_capacity(BUF_SIZE);
    let mut prev_was_cr = false;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }

        out.clear();
        for &b in &buf[..n] {
            if prev_was_cr && b != b'\n' {
                out.push(b'\r');
            }
            if b != b'\r' {
                out.push(b);
            }
            prev_was_cr = b == b'\r';
        }
        update(&out);
    }

    if prev_was_cr {
        update(b"\r");
    }

    Ok(())
}

/// Abstraction over SHA1 hasher implementations (gix and sha1 crate) so the
/// normalization algorithm is written exactly once. Both
/// [`hash_file_maybe_normalized`] and [`manual_hash_file_maybe_normalized`]
/// delegate to [`hash_file_normalized`].
trait BlobHasher: Sized {
    type Output;

    fn new() -> Self;
    fn write_blob_header(&mut self, blob_len: u64);
    fn update(&mut self, data: &[u8]);
    fn finalize(self) -> Result<Self::Output, std::io::Error>;
}

struct GixBlobHasher(gix_index::hash::Hasher);

impl BlobHasher for GixBlobHasher {
    type Output = gix_index::hash::ObjectId;

    fn new() -> Self {
        Self(gix_index::hash::hasher(gix_index::hash::Kind::Sha1))
    }

    fn write_blob_header(&mut self, blob_len: u64) {
        self.0.update(&gix_object::encode::loose_header(
            gix_object::Kind::Blob,
            blob_len,
        ));
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finalize(self) -> Result<gix_index::hash::ObjectId, std::io::Error> {
        self.0.try_finalize().map_err(std::io::Error::other)
    }
}

struct ManualBlobHasher(Sha1);

impl BlobHasher for ManualBlobHasher {
    type Output = OidHash;

    fn new() -> Self {
        Self(Sha1::new())
    }

    fn write_blob_header(&mut self, blob_len: u64) {
        self.0.update(b"blob ");
        self.0.update(blob_len.to_string().as_bytes());
        self.0.update([b'\0']);
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finalize(self) -> Result<OidHash, std::io::Error> {
        let result = self.0.finalize();
        let mut hex_buf = [0u8; 40];
        hex::encode_to_slice(result, &mut hex_buf).unwrap();
        Ok(OidHash::from_hex_buf(hex_buf))
    }
}

fn validate_file_type(
    path: &AbsoluteSystemPath,
    metadata: &std::fs::Metadata,
) -> Result<(), std::io::Error> {
    // Reject exotic file types (sockets, FIFOs, device nodes). Directories
    // pass through to fail naturally with a descriptive IsADirectory error.
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{path}: not a regular file"),
        ));
    }
    Ok(())
}

/// Hash a file as a git blob, applying CRLF→LF normalization when the
/// `text` attribute requires it.
///
/// For `TextAttr::Auto`, binary detection (NUL in first 8KB) is performed
/// during the fused scan+hash pass — no separate file open.
///
/// Single-pass for the common case (no normalization needed):
/// 1. Get file length from metadata
/// 2. Fused scan + speculative raw hash: write blob header, then scan for CRLFs
///    while simultaneously feeding raw bytes into the hasher
/// 3. If no normalization needed, return the raw hash (one read total)
///
/// Two-pass only when CRLF normalization is actually required:
/// 1. Fused scan + speculative raw hash (result discarded)
/// 2. Seek to start, write normalized blob header, stream with CRLF→LF
///
/// Memory bounded at ~128KB per call.
fn hash_file_normalized<H: BlobHasher>(
    file: &mut std::fs::File,
    file_len: u64,
    attr: TextAttr,
) -> Result<H::Output, std::io::Error> {
    // Fused scan + speculative raw hash in a single pass. The blob header
    // uses the raw file length; if normalization turns out to be needed we
    // discard this hash and do a second pass with the normalized length.
    let mut raw_hasher = H::new();
    raw_hasher.write_blob_header(file_len);
    let scan = scan_file_and_feed(file, attr == TextAttr::Auto, |data| {
        raw_hasher.update(data);
    })?;

    if !should_normalize(attr, &scan) {
        return raw_hasher.finalize();
    }

    // CRLF normalization required — second pass with the normalized length.
    // Safety: the file may have changed between the scan pass and this
    // normalization pass (TOCTOU). The length check below detects this.
    file.seek(SeekFrom::Start(0))?;
    let normalized_len = scan.byte_count.saturating_sub(scan.crlf_count);

    let mut hasher = H::new();
    hasher.write_blob_header(normalized_len);
    let mut bytes_hashed: u64 = 0;
    stream_normalized(file, |data| {
        bytes_hashed += data.len() as u64;
        hasher.update(data);
    })?;
    if bytes_hashed != normalized_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "CRLF normalization length mismatch: scan predicted {normalized_len}, stream \
                 produced {bytes_hashed}"
            ),
        ));
    }
    hasher.finalize()
}

/// Hash via the gix code path (used when `.git/` is present).
pub(crate) fn hash_file_maybe_normalized(
    path: &AbsoluteSystemPath,
    attr: TextAttr,
) -> Result<gix_index::hash::ObjectId, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    validate_file_type(path, &metadata)?;
    hash_file_normalized::<GixBlobHasher>(&mut file, metadata.len(), attr)
}

/// Hash via the manual code path (used after `turbo prune` removes `.git/`).
pub(crate) fn manual_hash_file_maybe_normalized(
    path: &AbsoluteSystemPath,
    attr: TextAttr,
) -> Result<OidHash, crate::Error> {
    let mut file = path.open()?;
    let metadata = file.metadata()?;
    validate_file_type(path, &metadata)?;
    Ok(hash_file_normalized::<ManualBlobHasher>(
        &mut file,
        metadata.len(),
        attr,
    )?)
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp, dir)
    }

    // -- scan_file tests --

    #[test]
    fn test_scan_empty_file() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("empty.txt");
        std::fs::write(path.as_std_path(), b"").unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, false).unwrap();
        assert_eq!(scan.byte_count, 0);
        assert_eq!(scan.crlf_count, 0);
        assert!(!scan.is_binary);
    }

    #[test]
    fn test_scan_pure_crlf() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("crlf.txt");
        std::fs::write(path.as_std_path(), b"a\r\nb\r\nc\r\n").unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, false).unwrap();
        assert_eq!(scan.byte_count, 9);
        assert_eq!(scan.crlf_count, 3);
    }

    #[test]
    fn test_scan_binary_with_crlf() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("binary.bin");
        std::fs::write(path.as_std_path(), [0x00, b'\r', b'\n', 0xFF]).unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, true).unwrap();
        assert_eq!(scan.byte_count, 4);
        assert_eq!(scan.crlf_count, 1);
        assert!(scan.is_binary);
    }

    #[test]
    fn test_scan_lone_cr() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("lone-cr.txt");
        std::fs::write(path.as_std_path(), b"hello\rworld\n").unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, false).unwrap();
        assert_eq!(scan.crlf_count, 0);
    }

    #[test]
    fn test_scan_old_mac_cr_only() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("old-mac.txt");
        std::fs::write(path.as_std_path(), b"hello\rworld\r").unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, false).unwrap();
        assert_eq!(scan.crlf_count, 0);
        assert_eq!(scan.byte_count, 12);
    }

    #[test]
    fn test_scan_binary_detect_nul_after_8kb_is_text() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("late-nul.bin");
        // 8KB of text followed by NUL — should be classified as text
        let mut content = vec![b'x'; GIT_BINARY_DETECT_LEN];
        content.push(0x00);
        std::fs::write(path.as_std_path(), &content).unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, true).unwrap();
        assert!(
            !scan.is_binary,
            "NUL after the first 8KB should not trigger binary detection"
        );
    }

    #[test]
    fn test_scan_binary_detect_nul_in_first_8kb() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("early-nul.bin");
        let mut content = vec![b'x'; 100];
        content.push(0x00);
        content.extend_from_slice(&[b'y'; 100]);
        std::fs::write(path.as_std_path(), &content).unwrap();
        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, true).unwrap();
        assert!(scan.is_binary);
    }

    #[test]
    fn test_scan_crlf_split_across_chunk_boundary() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("boundary.txt");
        // \r is the last byte of the first 64KB chunk, \n is the first
        // byte of the second chunk
        let mut content = vec![b'x'; BUF_SIZE - 1];
        content.push(b'\r');
        content.push(b'\n');
        content.extend_from_slice(b"after");
        std::fs::write(path.as_std_path(), &content).unwrap();

        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, false).unwrap();
        assert_eq!(
            scan.crlf_count, 1,
            "CRLF split across chunk boundary must be detected"
        );
        assert_eq!(scan.byte_count, (BUF_SIZE + 6) as u64);
    }

    #[test]
    fn test_scan_standalone_cr_at_chunk_boundary() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("boundary-cr.txt");
        // \r is the last byte of the first chunk, next byte is NOT \n
        let mut content = vec![b'x'; BUF_SIZE - 1];
        content.push(b'\r');
        content.push(b'y'); // not \n
        std::fs::write(path.as_std_path(), &content).unwrap();

        let mut f = std::fs::File::open(path.as_std_path()).unwrap();
        let scan = scan_file(&mut f, false).unwrap();
        assert_eq!(
            scan.crlf_count, 0,
            "standalone \\r at chunk boundary must not be counted as CRLF"
        );
    }

    // -- stream_normalized tests --

    #[test]
    fn test_stream_normalized_basic() {
        let input = b"a\r\nb\r\nc\r\n";
        let mut output = Vec::new();
        stream_normalized(&mut &input[..], |data| output.extend_from_slice(data)).unwrap();
        assert_eq!(output, b"a\nb\nc\n");
    }

    #[test]
    fn test_stream_normalized_preserves_standalone_cr() {
        let input = b"hello\rworld\n";
        let mut output = Vec::new();
        stream_normalized(&mut &input[..], |data| output.extend_from_slice(data)).unwrap();
        assert_eq!(output, b"hello\rworld\n");
    }

    #[test]
    fn test_stream_normalized_trailing_cr() {
        let input = b"data\r";
        let mut output = Vec::new();
        stream_normalized(&mut &input[..], |data| output.extend_from_slice(data)).unwrap();
        assert_eq!(output, b"data\r");
    }

    #[test]
    fn test_stream_normalized_chunk_boundary_crlf() {
        // \r at end of first chunk, \n at start of second
        let mut input = vec![b'x'; BUF_SIZE - 1];
        input.push(b'\r');
        input.push(b'\n');
        input.extend_from_slice(b"end");

        let mut output = Vec::new();
        stream_normalized(&mut &input[..], |data| output.extend_from_slice(data)).unwrap();

        let mut expected = vec![b'x'; BUF_SIZE - 1];
        expected.push(b'\n');
        expected.extend_from_slice(b"end");
        assert_eq!(
            output, expected,
            "CRLF split across chunk boundary must be normalized"
        );
    }

    #[test]
    fn test_stream_normalized_chunk_boundary_standalone_cr() {
        // \r at end of first chunk, NOT followed by \n
        let mut input = vec![b'x'; BUF_SIZE - 1];
        input.push(b'\r');
        input.push(b'y');

        let mut output = Vec::new();
        stream_normalized(&mut &input[..], |data| output.extend_from_slice(data)).unwrap();

        let mut expected = vec![b'x'; BUF_SIZE - 1];
        expected.push(b'\r');
        expected.push(b'y');
        assert_eq!(
            output, expected,
            "standalone \\r at chunk boundary must be preserved"
        );
    }

    // -- hash function tests --

    /// Verify that our normalized hashing produces OIDs identical to
    /// `git hash-object --path` (which applies .gitattributes filters).
    /// This is the ground-truth test for CRLF normalization correctness.
    #[test]
    fn test_normalized_hash_matches_git_hash_object_with_filters() {
        let (_tmp, root) = tmp_dir();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "--local", "core.autocrlf", "false"])
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        std::fs::write(root.as_std_path().join(".gitattributes"), "* text=auto\n").unwrap();

        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("pure-crlf.txt", b"a\r\nb\r\nc\r\n".to_vec()),
            ("mixed.txt", b"line1\nline2\r\nline3\n".to_vec()),
            ("no-crlf.txt", b"just lf\n".to_vec()),
            ("empty.txt", b"".to_vec()),
        ];

        for (name, content) in &cases {
            std::fs::write(root.as_std_path().join(name), content).unwrap();
        }

        for (name, _) in &cases {
            let output = std::process::Command::new("git")
                .args(["hash-object", "--path", name, "--stdin"])
                .stdin(std::process::Stdio::from(
                    std::fs::File::open(root.as_std_path().join(name)).unwrap(),
                ))
                .current_dir(root.as_std_path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git hash-object --path failed for {name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = String::from_utf8(output.stdout).unwrap();
            let expected = expected.trim();

            let path = root.join_component(name);
            let actual = hash_file_maybe_normalized(&path, TextAttr::Auto).unwrap();
            let mut hex_buf = [0u8; 40];
            hex::encode_to_slice(actual.as_bytes(), &mut hex_buf).unwrap();
            let actual_str = std::str::from_utf8(&hex_buf).unwrap();

            assert_eq!(
                actual_str, expected,
                "normalized hash for {name} must match git hash-object --path (with filters)"
            );
        }
    }

    /// Verify that the gix-based and sha1-based normalized hashers produce
    /// identical OIDs for a comprehensive set of inputs. This is critical
    /// because the git path uses gix and the manual path uses sha1 — they
    /// must always agree.
    #[test]
    fn test_gix_and_manual_normalized_hashes_agree() {
        let (_tmp, root) = tmp_dir();

        let mut boundary_content = vec![b'x'; BUF_SIZE - 1];
        boundary_content.push(b'\r');
        boundary_content.push(b'\n');
        boundary_content.extend_from_slice(b"after-boundary");

        let cases: Vec<(&str, Vec<u8>, TextAttr)> = vec![
            ("empty.txt", vec![], TextAttr::Set),
            ("lf-only.txt", b"line1\nline2\n".to_vec(), TextAttr::Set),
            ("pure-crlf.txt", b"a\r\nb\r\nc\r\n".to_vec(), TextAttr::Set),
            (
                "mixed.txt",
                b"line1\nline2\r\nline3\n".to_vec(),
                TextAttr::Set,
            ),
            ("trailing-cr.txt", b"data\r".to_vec(), TextAttr::Set),
            (
                "standalone-cr.txt",
                b"hello\rworld\n".to_vec(),
                TextAttr::Set,
            ),
            ("boundary-crlf.txt", boundary_content, TextAttr::Set),
            // Auto with text content
            (
                "auto-text.txt",
                b"hello\r\nworld\r\n".to_vec(),
                TextAttr::Auto,
            ),
            // Auto with binary content (NUL byte)
            (
                "auto-binary.bin",
                vec![0x00, b'\r', b'\n', 0xFF],
                TextAttr::Auto,
            ),
            // Unspecified — should hash raw
            ("unspec.txt", b"a\r\nb\r\n".to_vec(), TextAttr::Unspecified),
        ];

        for (name, content, attr) in &cases {
            let path = root.join_component(name);
            std::fs::write(path.as_std_path(), content).unwrap();

            let gix_result = hash_file_maybe_normalized(&path, *attr).unwrap();
            let manual_result = manual_hash_file_maybe_normalized(&path, *attr).unwrap();

            let mut hex_buf = [0u8; 40];
            hex::encode_to_slice(gix_result.as_bytes(), &mut hex_buf).unwrap();
            let gix_hex = std::str::from_utf8(&hex_buf).unwrap();

            assert_eq!(
                gix_hex, &*manual_result,
                "gix and manual hashes must agree for {name} (attr={attr:?})"
            );
        }
    }

    // -- GitAttrs tests --

    #[test]
    fn test_gitattrs_load_and_resolve() {
        let (_tmp, root) = tmp_dir();
        std::fs::write(
            root.as_std_path().join(".gitattributes"),
            "* text=auto\n*.png binary\n*.md text\n*.dat -text\n",
        )
        .unwrap();

        let attrs = GitAttrs::load(&root).expect("should load .gitattributes");

        assert_eq!(attrs.resolve_text_attr("README.md"), TextAttr::Set);
        assert_eq!(attrs.resolve_text_attr("src/index.ts"), TextAttr::Auto);
        assert_eq!(attrs.resolve_text_attr("image.png"), TextAttr::Unset);
        assert_eq!(attrs.resolve_text_attr("data.dat"), TextAttr::Unset);
    }

    #[test]
    fn test_gitattrs_returns_none_when_missing() {
        let (_tmp, root) = tmp_dir();
        assert!(GitAttrs::load(&root).is_none());
    }

    // -- hash_file_maybe_normalized edge-case tests --

    #[test]
    fn test_hash_binary_file_with_auto_is_raw() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("binary.bin");
        let content = vec![0x00, b'\r', b'\n', 0xFF, 0xFE];
        std::fs::write(path.as_std_path(), &content).unwrap();

        // With Auto, binary should be hashed raw (no normalization)
        let auto_result = hash_file_maybe_normalized(&path, TextAttr::Auto).unwrap();

        // With Unspecified, should also be raw
        let raw_result = hash_file_maybe_normalized(&path, TextAttr::Unspecified).unwrap();

        // Both should produce the same hash (raw bytes)
        assert_eq!(
            auto_result, raw_result,
            "binary file with Auto should hash the same as Unspecified (raw)"
        );
    }

    #[test]
    fn test_hash_text_auto_normalizes_crlf() {
        let (_tmp, root) = tmp_dir();
        let path = root.join_component("text.txt");
        std::fs::write(path.as_std_path(), b"a\r\nb\r\n").unwrap();

        let auto_hash = hash_file_maybe_normalized(&path, TextAttr::Auto).unwrap();
        let set_hash = hash_file_maybe_normalized(&path, TextAttr::Set).unwrap();

        // Both Auto and Set should normalize CRLF for a text file
        assert_eq!(auto_hash, set_hash);

        // Unspecified should hash raw (different from normalized)
        let raw_hash = hash_file_maybe_normalized(&path, TextAttr::Unspecified).unwrap();
        assert_ne!(
            auto_hash, raw_hash,
            "normalized hash should differ from raw for CRLF content"
        );
    }

    /// Verify chunk-boundary CRLF normalization against `git hash-object`.
    #[test]
    fn test_chunk_boundary_crlf_matches_git() {
        let (_tmp, root) = tmp_dir();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "--local", "core.autocrlf", "false"])
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        std::fs::write(root.as_std_path().join(".gitattributes"), "* text=auto\n").unwrap();

        // \r at end of first 64KB chunk, \n at start of second
        let mut content = vec![b'A'; BUF_SIZE - 1];
        content.push(b'\r');
        content.push(b'\n');
        content.extend_from_slice(b"tail");

        let name = "boundary.txt";
        std::fs::write(root.as_std_path().join(name), &content).unwrap();

        let output = std::process::Command::new("git")
            .args(["hash-object", "--path", name, "--stdin"])
            .stdin(std::process::Stdio::from(
                std::fs::File::open(root.as_std_path().join(name)).unwrap(),
            ))
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let expected = String::from_utf8(output.stdout).unwrap();
        let expected = expected.trim();

        let path = root.join_component(name);
        let actual = hash_file_maybe_normalized(&path, TextAttr::Auto).unwrap();
        let mut hex_buf = [0u8; 40];
        hex::encode_to_slice(actual.as_bytes(), &mut hex_buf).unwrap();
        let actual_str = std::str::from_utf8(&hex_buf).unwrap();

        assert_eq!(
            actual_str, expected,
            "chunk-boundary CRLF normalization must match git hash-object"
        );
    }
}
