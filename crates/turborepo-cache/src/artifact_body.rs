use std::{
    io::{Read, Seek, SeekFrom, Write},
    pin::Pin,
};

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

use crate::{
    CacheError,
    cache_archive::CacheWriter,
    signature_authentication::{ArtifactSignatureAuthenticator, SignatureError},
};

/// Artifacts smaller than this are kept in memory during transfers. Larger
/// ones roll over to an anonymous temporary file (unlinked on creation where
/// the platform supports it) so retained payload memory stays bounded no
/// matter how large the compressed artifact is.
pub(crate) const ARTIFACT_MEMORY_THRESHOLD: usize = 8 * 1024 * 1024;

/// 256 KB chunk size for reading/writing artifact bodies. Larger chunks
/// reduce per-chunk overhead (mutex locks in UploadProgress, hyper body
/// framing) which improves throughput for large artifacts compared to small
/// buffers.
pub(crate) const ARTIFACT_CHUNK_BYTES: usize = 256 * 1024;

/// The compressed artifact body, either in memory (small) or spooled to an
/// anonymous temporary file (large). A single built archive can be shared by
/// the local filesystem cache and the remote cache so identical bytes are
/// installed, signed, and uploaded exactly once.
pub(crate) enum ArtifactBody {
    /// Small artifact held in memory; retries are a cheap `Bytes` refcount
    /// bump.
    InMemory(bytes::Bytes),
    /// Large artifact spooled to an anonymous temporary file; consumers read
    /// it back from the start through fresh handles in bounded chunks.
    OnDisk(std::fs::File),
}

pub(crate) type UploadStream = Pin<
    Box<
        dyn futures::Stream<Item = Result<bytes::Bytes, turborepo_api_client::Error>> + Send + Sync,
    >,
>;

impl ArtifactBody {
    /// Builds the canonical compressed archive for `files` exactly once,
    /// spooling to disk when it grows past the in-memory threshold.
    pub(crate) fn from_files(
        anchor: &AbsoluteSystemPath,
        files: &[AnchoredSystemPathBuf],
    ) -> Result<Self, CacheError> {
        let mut spool = tempfile::spooled_tempfile(ARTIFACT_MEMORY_THRESHOLD);
        {
            let mut buffered = std::io::BufWriter::new(&mut spool);
            let mut cache_archive = CacheWriter::from_writer(&mut buffered, true)?;
            for file in files {
                cache_archive.add_file(anchor, file)?;
            }
            // finish() writes the tar footer; dropping then auto-finishes the
            // zstd stream into the buffer.
            cache_archive.finish()?;
            // BufWriter's Drop ignores flush errors; flush explicitly so a
            // failed final write cannot be reused as a truncated artifact.
            buffered.flush()?;
        }
        spool.seek(SeekFrom::Start(0))?;
        Self::from_spool(spool)
    }

    /// Converts a spooled artifact into its shared representation, keeping
    /// small artifacts in memory and large ones on disk.
    pub(crate) fn from_spool(mut spool: tempfile::SpooledTempFile) -> Result<Self, CacheError> {
        if spool.is_rolled() {
            let mut file = spool.into_file()?;
            // Rewind so the first consumer reads from the start.
            file.seek(SeekFrom::Start(0))?;
            Ok(ArtifactBody::OnDisk(file))
        } else {
            let mut bytes = Vec::new();
            spool.read_to_end(&mut bytes)?;
            Ok(ArtifactBody::InMemory(bytes::Bytes::from(bytes)))
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            ArtifactBody::InMemory(bytes) => bytes.len(),
            ArtifactBody::OnDisk(file) => {
                file.metadata().map(|meta| meta.len() as usize).unwrap_or(0)
            }
        }
    }

    pub(crate) fn generate_tag(
        &self,
        signer: &ArtifactSignatureAuthenticator,
        hash: &str,
    ) -> Result<String, SignatureError> {
        match self {
            ArtifactBody::InMemory(bytes) => signer.generate_tag(hash.as_bytes(), bytes),
            ArtifactBody::OnDisk(file) => {
                signer.generate_tag_reader(hash.as_bytes(), file, self.len() as u64)
            }
        }
    }

    /// A fresh reader positioned at the start of the archive, independent of
    /// where previous consumers stopped. Readers are cheap: a `Bytes`
    /// refcount bump in memory or a `dup` of the spool file on disk. The
    /// on-disk variant is buffered so consumers issuing small reads (like
    /// tar header parsing) do not turn into per-block syscalls.
    pub(crate) fn reader(&self) -> std::io::Result<Box<dyn Read + Send>> {
        match self {
            ArtifactBody::InMemory(bytes) => Ok(Box::new(std::io::Cursor::new(bytes.clone()))),
            ArtifactBody::OnDisk(file) => {
                let mut handle = file.try_clone()?;
                handle.seek(SeekFrom::Start(0))?;
                Ok(Box::new(std::io::BufReader::with_capacity(
                    ARTIFACT_CHUNK_BYTES,
                    handle,
                )))
            }
        }
    }

    /// Copies the complete archive bytes to `writer`, reading from the start
    /// in bounded chunks regardless of where previous consumers stopped.
    pub(crate) fn copy_to(&self, mut writer: impl Write) -> Result<(), CacheError> {
        std::io::copy(&mut self.reader()?, &mut writer)?;
        Ok(())
    }

    /// A fresh bounded stream over the archive bytes, so every upload attempt
    /// sends byte-identical content.
    pub(crate) fn stream(&self) -> turborepo_api_client::Result<UploadStream> {
        match self {
            ArtifactBody::InMemory(bytes) => Ok(Box::pin(chunked_byte_stream(
                bytes.clone(),
                ARTIFACT_CHUNK_BYTES,
            ))),
            ArtifactBody::OnDisk(file) => {
                // `try_clone` hands us an independent handle; seek it to the
                // start because signing (and any previous attempt) moved the
                // shared offset.
                let mut handle = file.try_clone()?;
                handle.seek(SeekFrom::Start(0))?;
                let reader = tokio_util::io::ReaderStream::with_capacity(
                    tokio::fs::File::from_std(handle),
                    ARTIFACT_CHUNK_BYTES,
                );
                Ok(Box::pin(futures::StreamExt::map(reader, |chunk| {
                    chunk.map_err(turborepo_api_client::Error::from)
                })))
            }
        }
    }
}

/// Yields zero-copy `Bytes` slices of `chunk_size` from an already-in-memory
/// buffer. Each `.slice()` call is O(1) -- it bumps the `Bytes` refcount
/// rather than copying data.
fn chunked_byte_stream(
    buf: bytes::Bytes,
    chunk_size: usize,
) -> impl futures::Stream<Item = Result<bytes::Bytes, turborepo_api_client::Error>> {
    let len = buf.len();
    futures::stream::unfold((buf, 0usize), move |(buf, offset)| async move {
        if offset >= len {
            return None;
        }
        let end = (offset + chunk_size).min(len);
        let chunk = buf.slice(offset..end);
        Some((Ok(chunk), (buf, end)))
    })
}
