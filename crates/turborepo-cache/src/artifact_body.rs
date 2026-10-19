use std::{
    io::{Read, Seek, SeekFrom, Write},
    pin::Pin,
    sync::Arc,
};

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

use crate::{
    CacheError,
    cache_archive::{CacheReader, CacheWriter, RestoreManifest},
    signature_authentication::{ArtifactSignatureAuthenticator, SignatureError},
};

pub(crate) struct Artifact {
    body: Arc<ArtifactBody>,
    manifest: RestoreManifest,
}

impl Artifact {
    pub(crate) fn from_files(
        anchor: &AbsoluteSystemPath,
        files: &[AnchoredSystemPathBuf],
    ) -> Result<Self, CacheError> {
        let mut cache_archive = CacheWriter::spooled(ARTIFACT_MEMORY_THRESHOLD)?;
        let manifest = cache_archive.add_files(anchor, files)?;
        let mut spool = cache_archive.finish_spooled()?;
        spool.seek(SeekFrom::Start(0))?;
        Ok(Self {
            body: Arc::new(ArtifactBody::from_spool(spool)?),
            manifest,
        })
    }

    pub(crate) fn body(&self) -> &Arc<ArtifactBody> {
        &self.body
    }

    pub(crate) fn manifest(&self) -> &RestoreManifest {
        &self.manifest
    }
}

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
    pub(crate) fn restore(
        self: &Arc<Self>,
        anchor: &AbsoluteSystemPath,
    ) -> Result<(Vec<AnchoredSystemPathBuf>, Artifact), CacheError> {
        let mut cache_reader = CacheReader::from_reader(self.reader()?, true)?;
        let (files, manifest) = cache_reader.restore(anchor, None)?;
        Ok((
            files,
            Artifact {
                body: Arc::clone(self),
                manifest,
            },
        ))
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
            ArtifactBody::OnDisk(_) => {
                signer.generate_tag_reader(hash.as_bytes(), self.reader()?, self.len() as u64)
            }
        }
    }

    /// A fresh reader positioned at the start of the archive, independent of
    /// every other reader, so several consumers can read at once. Readers are
    /// cheap: a `Bytes` refcount bump in memory or a `dup` of the spool file
    /// on disk, read with positional reads at the reader's own offset. The
    /// on-disk variant is buffered so consumers issuing small reads (like
    /// tar header parsing) do not turn into per-block syscalls.
    pub(crate) fn reader(&self) -> std::io::Result<Box<dyn Read + Send>> {
        match self {
            ArtifactBody::InMemory(bytes) => Ok(Box::new(std::io::Cursor::new(bytes.clone()))),
            ArtifactBody::OnDisk(file) => Ok(Box::new(std::io::BufReader::with_capacity(
                ARTIFACT_CHUNK_BYTES,
                PositionalReader::new(file)?,
            ))),
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
                // Positional reads keep every upload attempt independent of
                // other readers, like the local cache install that runs at
                // the same time.
                let reader = PositionalReader::new(file)?;
                let chunks = futures::stream::try_unfold(reader, |mut reader| async move {
                    let (reader, chunk) = tokio::task::spawn_blocking(move || {
                        let mut chunk = vec![0; ARTIFACT_CHUNK_BYTES];
                        let len = reader.read(&mut chunk)?;
                        chunk.truncate(len);
                        Ok::<_, std::io::Error>((reader, chunk))
                    })
                    .await
                    .map_err(std::io::Error::other)??;
                    Ok::<_, std::io::Error>(
                        (!chunk.is_empty()).then(|| (bytes::Bytes::from(chunk), reader)),
                    )
                });
                Ok(Box::pin(futures::StreamExt::map(chunks, |chunk| {
                    chunk.map_err(turborepo_api_client::Error::from)
                })))
            }
        }
    }
}

/// Reads a file from its own offset with positional reads. Duplicated file
/// handles share one OS file position, so readers that seek and read would
/// interfere when they run at the same time; these don't.
struct PositionalReader {
    file: std::fs::File,
    offset: u64,
}

impl PositionalReader {
    fn new(file: &std::fs::File) -> std::io::Result<Self> {
        Ok(Self {
            file: file.try_clone()?,
            offset: 0,
        })
    }
}

impl Read for PositionalReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = read_at(&self.file, buf, self.offset)?;
        self.offset += len as u64;
        Ok(len)
    }
}

#[cfg(unix)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    // Moves the shared file position, which no reader relies on.
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(not(any(unix, windows)))]
fn read_at(mut file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    // Without a positional read, make seek + read atomic across readers.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    file.seek(SeekFrom::Start(offset))?;
    file.read(buf)
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use anyhow::Result;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    /// Archives `files` with the single-pass writer, for comparison.
    fn single_pass_archive(
        anchor: &AbsoluteSystemPath,
        files: &[AnchoredSystemPathBuf],
    ) -> Result<Vec<u8>> {
        let mut archive = Vec::new();
        {
            let mut cache_archive = CacheWriter::from_writer(&mut archive, true)?;
            cache_archive.add_files(anchor, files)?;
            cache_archive.finish()?;
        }
        Ok(archive)
    }

    #[test]
    fn from_files_matches_single_pass_archive() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let anchor = AbsoluteSystemPathBuf::try_from(dir.path())?;
        let dist = anchor.join_component("dist");
        dist.create_dir_all()?;

        // Compressible output: compressed on a worker thread, small enough
        // compressed to stay in memory.
        let text: String = (0..300_000)
            .map(|line| format!("export const value_{line} = {};\n", line * 31 % 977))
            .collect();
        dist.join_component("bundle.js")
            .create_with_contents(&text)?;
        let files = vec![
            AnchoredSystemPathBuf::from_raw("dist")?,
            AnchoredSystemPathBuf::from_raw("dist/bundle.js")?,
        ];
        let artifact = Artifact::from_files(&anchor, &files)?;
        let body = artifact.body().as_ref();
        assert!(matches!(body, ArtifactBody::InMemory(_)));
        let mut built = Vec::new();
        body.reader()?.read_to_end(&mut built)?;
        assert!(built == single_pass_archive(&anchor, &files)?);

        // Incompressible output: large enough compressed to spill to disk.
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let noise: Vec<u8> = (0..ARTIFACT_MEMORY_THRESHOLD + (2 << 20))
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        dist.join_component("blob.bin")
            .create_with_contents(&noise)?;
        let files = vec![
            AnchoredSystemPathBuf::from_raw("dist")?,
            AnchoredSystemPathBuf::from_raw("dist/blob.bin")?,
            AnchoredSystemPathBuf::from_raw("dist/bundle.js")?,
        ];
        let artifact = Artifact::from_files(&anchor, &files)?;
        let body = artifact.body().as_ref();
        assert!(matches!(body, ArtifactBody::OnDisk(_)));
        let mut built = Vec::new();
        body.reader()?.read_to_end(&mut built)?;
        assert!(built == single_pass_archive(&anchor, &files)?);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_readers_of_disk_artifact_see_whole_body() -> Result<()> {
        let mut spool = tempfile::spooled_tempfile(ARTIFACT_MEMORY_THRESHOLD);
        let bytes: Vec<u8> = (0..ARTIFACT_MEMORY_THRESHOLD + 3 * ARTIFACT_CHUNK_BYTES + 17)
            .map(|i| (i % 251) as u8)
            .collect();
        spool.write_all(&bytes)?;
        spool.rewind()?;
        let body = std::sync::Arc::new(ArtifactBody::from_spool(spool)?);
        assert!(matches!(*body, ArtifactBody::OnDisk(_)));

        // Local installs copy the body on blocking threads while the upload
        // streams it.
        let copies: Vec<_> = (0..3)
            .map(|_| {
                let body = body.clone();
                tokio::task::spawn_blocking(move || {
                    let mut copy = Vec::new();
                    body.copy_to(&mut copy).map(|()| copy)
                })
            })
            .collect();
        let mut streamed = Vec::new();
        let mut stream = body.stream()?;
        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
            streamed.extend_from_slice(&chunk?);
        }
        assert!(streamed == bytes);
        for copy in copies {
            assert!(copy.await?? == bytes);
        }
        Ok(())
    }

    #[test]
    fn signing_disk_artifact_after_local_copy_includes_body() -> Result<()> {
        let mut spool = tempfile::spooled_tempfile(ARTIFACT_MEMORY_THRESHOLD);
        let bytes = vec![0x5a; ARTIFACT_MEMORY_THRESHOLD + 1];
        spool.write_all(&bytes)?;
        spool.rewind()?;
        let body = ArtifactBody::from_spool(spool)?;
        assert!(matches!(body, ArtifactBody::OnDisk(_)));

        // The local cache consumes the body before the remote cache signs it.
        let mut local_archive = Vec::new();
        body.copy_to(&mut local_archive)?;
        assert_eq!(local_archive, bytes);

        let signer = ArtifactSignatureAuthenticator::new(
            b"team".to_vec(),
            Some(b"signature-secret-key-long-enough".to_vec()),
        );
        let tag = body.generate_tag(&signer, "hash")?;
        assert!(signer.validate(b"hash", &local_archive, &tag)?);
        assert_eq!(tag, signer.generate_tag(b"hash", &bytes)?);

        Ok(())
    }
}
