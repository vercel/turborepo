use std::{backtrace::Backtrace, collections::HashMap, io::Read};

use petgraph::graph::DiGraph;
use sha2::{Digest, Sha512};
use tar::Entry;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    CacheError,
    cache_archive::{
        restore_directory::{CachedDirTree, restore_directory},
        restore_manifest::RestoreManifest,
        restore_regular::restore_regular,
        restore_symlink::{
            canonicalize_linkname, restore_symlink, restore_symlink_allow_missing_target,
        },
    },
};

pub struct CacheReader<'a> {
    reader: Box<dyn Read + 'a>,
}

impl<'a> CacheReader<'a> {
    pub fn from_reader(reader: impl Read + 'a, is_compressed: bool) -> Result<Self, CacheError> {
        let reader: Box<dyn Read> = if is_compressed {
            Box::new(zstd::Decoder::new(reader)?)
        } else {
            Box::new(reader)
        };

        Ok(CacheReader { reader })
    }

    pub fn open(path: &AbsoluteSystemPathBuf) -> Result<Self, CacheError> {
        let _span = tracing::info_span!("cache_reader_open").entered();
        let file = path.open()?;
        let is_compressed = path.extension() == Some("zst");

        let reader: Box<dyn Read> = if is_compressed {
            Box::new(zstd::Decoder::new(file)?)
        } else {
            Box::new(file)
        };

        Ok(CacheReader { reader })
    }

    pub fn get_sha(mut self) -> Result<Vec<u8>, CacheError> {
        let mut hasher = Sha512::new();
        let mut buffer = [0; 8192];
        loop {
            let n = self.reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        Ok(hasher.finalize().to_vec())
    }

    pub fn restore(
        &mut self,
        anchor: &AbsoluteSystemPath,
        previous_manifest: Option<&RestoreManifest>,
    ) -> Result<(Vec<AnchoredSystemPathBuf>, RestoreManifest), CacheError> {
        let _span = tracing::info_span!("cache_reader_restore").entered();
        let mut restored = Vec::new();
        let mut new_manifest = RestoreManifest::new();
        anchor.create_dir_all()?;

        let dir_cache = CachedDirTree::new(anchor.to_owned());
        let mut tr = tar::Archive::new(&mut self.reader);

        Self::restore_entries(
            &mut tr,
            &mut restored,
            dir_cache,
            anchor,
            previous_manifest,
            &mut new_manifest,
        )?;
        Ok((restored, new_manifest))
    }

    fn restore_entries<T: Read>(
        tr: &mut tar::Archive<T>,
        restored: &mut Vec<AnchoredSystemPathBuf>,
        mut dir_cache: CachedDirTree,
        anchor: &AbsoluteSystemPath,
        previous_manifest: Option<&RestoreManifest>,
        new_manifest: &mut RestoreManifest,
    ) -> Result<(), CacheError> {
        let mut symlinks = Vec::new();

        for entry in tr.entries()? {
            let mut entry = entry?;
            let entry_type = entry.header().entry_type();
            match restore_entry(&mut dir_cache, anchor, &mut entry, previous_manifest) {
                Err(CacheError::LinkTargetDoesNotExist(_, _)) => {
                    symlinks.push(entry);
                }
                Err(e) => return Err(e),
                Ok((restored_path, skipped)) => {
                    if entry_type == tar::EntryType::Regular {
                        let key = restored_path.as_str().to_owned();
                        if skipped {
                            if let Some(existing) =
                                previous_manifest.and_then(|m| m.files.get(&key))
                            {
                                new_manifest.order.push(key.clone());
                                new_manifest.files.insert(key, *existing);
                            }
                        } else {
                            let resolved = anchor.resolve(&restored_path);
                            let _ = new_manifest.record_file(key, &resolved);
                        }
                    }
                    restored.push(restored_path);
                }
            }
        }

        let mut restored_symlinks =
            Self::topologically_restore_symlinks(&mut dir_cache, anchor, &symlinks)?;
        restored.append(&mut restored_symlinks);
        Ok(())
    }

    fn topologically_restore_symlinks<T: Read>(
        dir_cache: &mut CachedDirTree,
        anchor: &AbsoluteSystemPath,
        symlinks: &[Entry<'_, T>],
    ) -> Result<Vec<AnchoredSystemPathBuf>, CacheError> {
        let mut graph = DiGraph::new();
        let mut entry_lookup = HashMap::new();
        let mut restored = Vec::new();
        let mut nodes = HashMap::new();

        for entry in symlinks {
            let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;
            let processed_sourcename =
                canonicalize_linkname(anchor, &processed_name, processed_name.as_path())?;
            // symlink must have a linkname
            let linkname = entry.link_name()?.expect("symlink without linkname");

            let processed_linkname = canonicalize_linkname(anchor, &processed_name, &linkname)?;

            let source_node = *nodes
                .entry(processed_sourcename.clone())
                .or_insert_with(|| graph.add_node(processed_sourcename.clone()));
            let link_node = *nodes
                .entry(processed_linkname.clone())
                .or_insert_with(|| graph.add_node(processed_linkname.clone()));

            graph.add_edge(source_node, link_node, ());

            entry_lookup.insert(processed_sourcename, entry);
        }

        let nodes = petgraph::algo::toposort(&graph, None)
            .map_err(|_| CacheError::CycleDetected(Backtrace::capture()))?;

        for node in nodes {
            let key = &graph[node];

            let Some(entry) = entry_lookup.get(key) else {
                continue;
            };
            let file = restore_symlink_allow_missing_target(dir_cache, anchor, entry)?;
            restored.push(file);
        }

        Ok(restored)
    }
}

/// Returns `(path, skipped)` where `skipped` is true only for regular
/// files that matched the manifest and were not rewritten.
fn restore_entry<T: Read>(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &mut Entry<T>,
    manifest: Option<&RestoreManifest>,
) -> Result<(AnchoredSystemPathBuf, bool), CacheError> {
    let header = entry.header();

    match header.entry_type() {
        tar::EntryType::Directory => {
            restore_directory(dir_cache, anchor, entry).map(|p| (p, false))
        }
        tar::EntryType::Regular => restore_regular(dir_cache, anchor, entry, manifest),
        tar::EntryType::Symlink => restore_symlink(dir_cache, anchor, entry).map(|p| (p, false)),
        ty => Err(CacheError::RestoreUnsupportedFileType(
            ty,
            Backtrace::capture(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, fs::File, io::empty, path::Path};

    use anyhow::Result;
    use tar::Header;
    use tempfile::{TempDir, tempdir};
    use test_case::test_case;
    use tracing::debug;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use crate::cache_archive::{restore::CacheReader, restore_symlink::canonicalize_linkname};

    enum RawTarEntry {
        File {
            path: &'static str,
            body: Vec<u8>,
        },
        Directory {
            path: &'static str,
        },
        Symlink {
            link_path: &'static str,
            link_target: &'static str,
        },
    }

    fn generate_raw_tar(entries: &[RawTarEntry]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);
            for entry in entries {
                match entry {
                    RawTarEntry::File { path, body } => {
                        let mut header = Header::new_gnu();
                        header.set_size(body.len() as u64);
                        header.set_entry_type(tar::EntryType::Regular);
                        header.set_mode(0o644);
                        builder.append_data(&mut header, path, &body[..]).unwrap();
                    }
                    RawTarEntry::Directory { path } => {
                        let mut header = Header::new_gnu();
                        header.set_entry_type(tar::EntryType::Directory);
                        header.set_size(0);
                        header.set_mode(0o755);
                        builder.append_data(&mut header, path, empty()).unwrap();
                    }
                    RawTarEntry::Symlink {
                        link_path,
                        link_target,
                    } => {
                        let mut header = Header::new_gnu();
                        header.set_entry_type(tar::EntryType::Symlink);
                        header.set_size(0);
                        builder
                            .append_link(&mut header, link_path, link_target)
                            .unwrap();
                    }
                }
            }
            builder.into_inner().unwrap();
        }
        buf
    }

    // The `tar` crate's Builder rejects absolute paths and `..` segments.
    // To test that our restore code handles these, we write raw tar bytes
    // with the path injected directly into the header, bypassing builder
    // validation.
    fn generate_raw_tar_with_unsafe_path(path: &str, body: &[u8]) -> Vec<u8> {
        use std::io::Write;

        let mut buf = Vec::new();

        let mut header_bytes = [0u8; 512];

        // Write name (first 100 bytes of header)
        let name_bytes = path.as_bytes();
        let copy_len = name_bytes.len().min(100);
        header_bytes[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

        // Mode at offset 100 (8 bytes, octal ASCII)
        header_bytes[100..107].copy_from_slice(b"0000644");

        // UID at offset 108
        header_bytes[108..115].copy_from_slice(b"0001000");

        // GID at offset 116
        header_bytes[116..123].copy_from_slice(b"0001000");

        // Size at offset 124 (11 octal digits + null)
        let size_str = format!("{:011o}", body.len());
        header_bytes[124..135].copy_from_slice(size_str.as_bytes());

        // Mtime at offset 136
        header_bytes[136..147].copy_from_slice(b"00000000000");

        // Typeflag at offset 156: '0' = regular file
        header_bytes[156] = b'0';

        // USTAR magic at offset 257
        header_bytes[257..263].copy_from_slice(b"ustar\0");
        // Version at offset 263
        header_bytes[263..265].copy_from_slice(b"00");

        // Compute checksum: sum of all bytes in header, treating
        // the checksum field (offset 148-155) as spaces
        header_bytes[148..156].copy_from_slice(b"        ");
        let checksum: u32 = header_bytes.iter().map(|&b| b as u32).sum();
        let checksum_str = format!("{:06o}\0 ", checksum);
        header_bytes[148..156].copy_from_slice(checksum_str.as_bytes());

        buf.write_all(&header_bytes).unwrap();

        // Write body padded to 512 bytes
        buf.write_all(body).unwrap();
        let padding = 512 - (body.len() % 512);
        if padding < 512 {
            buf.write_all(&vec![0u8; padding]).unwrap();
        }

        // End-of-archive: two 512-byte blocks of zeros
        buf.write_all(&[0u8; 1024]).unwrap();

        buf
    }

    // Expected output of the cache
    #[derive(Debug)]
    struct ExpectedOutput(Vec<AnchoredSystemPathBuf>);

    enum TarFile {
        File {
            body: Vec<u8>,
            path: AnchoredSystemPathBuf,
        },
        Directory {
            path: AnchoredSystemPathBuf,
        },
        Symlink {
            // The path of the symlink itself
            link_path: AnchoredSystemPathBuf,
            // The target of the symlink
            link_target: AnchoredSystemPathBuf,
        },
        Fifo {
            path: AnchoredSystemPathBuf,
        },
    }

    struct TestCase {
        name: &'static str,
        // The files we start with
        input_files: Vec<TarFile>,
        // The expected files (there will be more files than `expected_output`
        // since we want to check entries of symlinked directories)
        expected_files: Vec<TarFile>,
        // What we expect to get from CacheArchive::restore, either a
        // Vec of restored files, or an error (represented as a string)
        expected_output: Result<Vec<AnchoredSystemPathBuf>, String>,
    }

    fn generate_tar(test_dir: &TempDir, files: &[TarFile]) -> Result<AbsoluteSystemPathBuf> {
        let test_archive_path = test_dir.path().join("test.tar");
        let archive_file = File::create(&test_archive_path)?;

        let mut tar_writer = tar::Builder::new(archive_file);

        for file in files {
            match file {
                TarFile::File { path, body } => {
                    debug!("Adding file: {:?}", path);
                    let mut header = Header::new_gnu();
                    header.set_size(body.len() as u64);
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_mode(0o644);
                    tar_writer.append_data(&mut header, path, &body[..])?;
                }
                TarFile::Directory { path } => {
                    debug!("Adding directory: {:?}", path);
                    let mut header = Header::new_gnu();
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_mode(0o755);
                    tar_writer.append_data(&mut header, path, empty())?;
                }
                TarFile::Symlink {
                    link_path: link_file,
                    link_target,
                } => {
                    debug!("Adding symlink: {:?} -> {:?}", link_file, link_target);
                    let mut header = tar::Header::new_gnu();
                    header.set_username("foo")?;
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_size(0);

                    tar_writer.append_link(&mut header, link_file, link_target)?;
                }
                // We don't support this, but we need to add it to a tar for testing purposes
                TarFile::Fifo { path } => {
                    let mut header = tar::Header::new_gnu();
                    header.set_entry_type(tar::EntryType::Fifo);
                    header.set_size(0);
                    tar_writer.append_data(&mut header, path, empty())?;
                }
            }
        }

        tar_writer.into_inner()?;

        Ok(AbsoluteSystemPathBuf::new(
            test_archive_path.to_string_lossy(),
        )?)
    }

    fn compress_tar(archive_path: &AbsoluteSystemPathBuf) -> Result<AbsoluteSystemPathBuf> {
        let mut input_file = File::open(archive_path)?;

        let output_file_path = format!("{archive_path}.zst");
        let output_file = File::create(&output_file_path)?;

        let mut zw = zstd::stream::Encoder::new(output_file, 0)?;
        std::io::copy(&mut input_file, &mut zw)?;

        zw.finish()?;

        Ok(AbsoluteSystemPathBuf::new(output_file_path)?)
    }

    fn assert_file_exists(anchor: &AbsoluteSystemPath, disk_file: &TarFile) -> Result<()> {
        match disk_file {
            TarFile::File { path, body } => {
                let full_name = anchor.resolve(path);
                debug!("reading {}", full_name);
                let file_contents = fs::read(full_name)?;

                assert_eq!(file_contents, *body);
            }
            TarFile::Directory { path } => {
                let full_name = anchor.resolve(path);
                let metadata = fs::metadata(full_name)?;

                assert!(metadata.is_dir());
            }
            TarFile::Symlink {
                link_path: link_file,
                link_target: expected_link_target,
            } => {
                let full_link_file = anchor.resolve(link_file);
                let link_target = fs::read_link(full_link_file)?;

                assert_eq!(link_target, expected_link_target.as_path().to_path_buf());
            }
            TarFile::Fifo { .. } => unreachable!("FIFOs are not supported"),
        }

        Ok(())
    }

    fn into_anchored_system_path_vec(items: Vec<&'static str>) -> Vec<AnchoredSystemPathBuf> {
        items
            .into_iter()
            .map(|item| AnchoredSystemPathBuf::try_from(Path::new(item)).unwrap())
            .collect()
    }

    #[test]
    fn test_name_traversal() -> Result<()> {
        let uncompressed_tar = include_bytes!("../../fixtures/name-traversal.tar");
        let compressed_tar = include_bytes!("../../fixtures/name-traversal.tar.zst");
        for (tar_bytes, is_compressed) in
            [(&uncompressed_tar[..], false), (&compressed_tar[..], true)]
        {
            let mut cache_reader = CacheReader::from_reader(tar_bytes, is_compressed)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;
            let result = cache_reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Invalid file path: path is malformed: ../escape"
            );
        }

        Ok(())
    }

    #[test]
    fn test_windows_unsafe() -> Result<()> {
        let uncompressed_tar = include_bytes!("../../fixtures/windows-unsafe.tar");
        let compressed_tar = include_bytes!("../../fixtures/windows-unsafe.tar.zst");

        for (tar_bytes, is_compressed) in
            [(&uncompressed_tar[..], false), (&compressed_tar[..], true)]
        {
            let mut cache_reader = CacheReader::from_reader(tar_bytes, is_compressed)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;
            let result = cache_reader.restore(anchor, None).map(|(f, _)| f);
            #[cfg(windows)]
            {
                assert!(result.is_err());
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "Invalid file path: Path is not safe for windows: \
                     windows-unsafe/this\\is\\a\\file\\on\\unix"
                );
            }
            #[cfg(unix)]
            {
                assert!(result.is_ok());
                let path = result.unwrap().pop().unwrap();
                assert_eq!(path.as_str(), "windows-unsafe/this\\is\\a\\file\\on\\unix");
            }
        }

        Ok(())
    }

    #[test]
    fn test_restore() -> Result<()> {
        let tests = vec![
            TestCase {
                name: "cache optimized",
                input_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-one").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-two").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/file").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/file").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-one").unwrap(),
                        body: vec![],
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-two").unwrap(),
                        body: vec![],
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/file").unwrap(),
                        body: vec![],
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/file").unwrap(),
                        body: vec![],
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec![
                    "one",
                    "one/two",
                    "one/two/three",
                    "one/two/a",
                    "one/two/three/file-one",
                    "one/two/three/file-two",
                    "one/two/a/file",
                    "one/two/b",
                    "one/two/b/file",
                ])),
            },
            TestCase {
                name: "pathological cache works",
                input_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/file").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/file").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-one").unwrap(),
                    },
                    TarFile::File {
                        body: vec![],
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-two").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-one").unwrap(),
                        body: vec![],
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/three/file-two").unwrap(),
                        body: vec![],
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/a/file").unwrap(),
                        body: vec![],
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("one/two/b/file").unwrap(),
                        body: vec![],
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec![
                    "one",
                    "one/two",
                    "one/two/a",
                    "one/two/b",
                    "one/two/three",
                    "one/two/a/file",
                    "one/two/b/file",
                    "one/two/three/file-one",
                    "one/two/three/file-two",
                ])),
            },
            TestCase {
                name: "symlink hello world",
                input_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("source").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("source").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec!["target", "source"])),
            },
            TestCase {
                name: "nested file",
                input_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("folder/").unwrap(),
                    },
                    TarFile::File {
                        body: b"file".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("folder/file").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("folder/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("folder/file").unwrap(),
                        body: b"file".to_vec(),
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec!["folder", "folder/file"])),
            },
            TestCase {
                name: "nested symlink",
                input_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("folder/").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("folder/symlink").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("../").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("folder/symlink/folder-sibling")
                            .unwrap(),
                        body: b"folder-sibling".to_vec(),
                    },
                ],
                expected_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("folder/").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("folder/symlink").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("../").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("folder/symlink/folder-sibling")
                            .unwrap(),
                        body: b"folder-sibling".to_vec(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("folder-sibling").unwrap(),
                        body: b"folder-sibling".to_vec(),
                    },
                ],
                #[cfg(unix)]
                expected_output: Ok(into_anchored_system_path_vec(vec![
                    "folder",
                    "folder/symlink",
                    "folder/symlink/folder-sibling",
                ])),
                #[cfg(windows)]
                expected_output: Err("IO error: The filename, directory name, or volume label \
                                      syntax is incorrect. (os error 123)"
                    .to_string()),
            },
            TestCase {
                name: "pathological symlinks",
                input_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                    TarFile::File {
                        body: b"real".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                        body: b"real".to_vec(),
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec![
                    "real", "one", "two", "three",
                ])),
            },
            TestCase {
                name: "place file at dir location",
                input_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("folder-not-file/").unwrap(),
                    },
                    TarFile::File {
                        body: b"subfile".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("folder-not-file/subfile").unwrap(),
                    },
                    TarFile::File {
                        body: b"this shouldn't work".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("folder-not-file").unwrap(),
                    },
                ],

                expected_files: vec![
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("folder-not-file/").unwrap(),
                    },
                    TarFile::File {
                        body: b"subfile".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("folder-not-file/subfile").unwrap(),
                    },
                ],
                #[cfg(unix)]
                expected_output: Err("IO error: Is a directory (os error 21)".to_string()),
                #[cfg(windows)]
                expected_output: Err("IO error: Access is denied. (os error 5)".to_string()),
            },
            TestCase {
                name: "symlink cycle",
                input_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                    },
                ],
                expected_files: vec![],
                expected_output: Err("links in the cache are cyclic".to_string()),
            },
            TestCase {
                name: "symlink clobber",
                input_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                    TarFile::File {
                        body: b"real".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                    TarFile::File {
                        body: b"real".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec!["real", "one"])),
            },
            TestCase {
                name: "symlink traversal",
                input_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("escape").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("../").unwrap(),
                    },
                    TarFile::File {
                        body: b"file".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("escape/file").unwrap(),
                    },
                ],
                expected_files: vec![TarFile::Symlink {
                    link_path: AnchoredSystemPathBuf::from_raw("escape").unwrap(),
                    link_target: AnchoredSystemPathBuf::from_raw("../").unwrap(),
                }],
                expected_output: Err("tar attempts to write outside of directory: ../".to_string()),
            },
            TestCase {
                name: "Double indirection: file",
                input_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("up").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("../").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("link").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("up").unwrap(),
                    },
                    TarFile::File {
                        body: b"file".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("link/outside-file").unwrap(),
                    },
                ],
                expected_files: vec![],
                expected_output: Err("tar attempts to write outside of directory: ../".to_string()),
            },
            TestCase {
                name: "Double indirection: folder",
                input_files: vec![
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("up").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("../").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("link").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("up").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("link/level-one/level-two").unwrap(),
                    },
                ],
                expected_files: vec![],
                expected_output: Err("tar attempts to write outside of directory: ../".to_string()),
            },
            TestCase {
                name: "fifo (and others) unsupported",
                input_files: vec![TarFile::Fifo {
                    path: AnchoredSystemPathBuf::from_raw("fifo").unwrap(),
                }],
                expected_files: vec![],
                expected_output: Err("attempted to restore unsupported file type: Fifo".to_string()),
            },
            TestCase {
                name: "duplicate restores",
                input_files: vec![
                    TarFile::File {
                        body: b"target".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("source").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/").unwrap(),
                    },
                ],
                expected_files: vec![
                    TarFile::File {
                        body: b"target".to_vec(),
                        path: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("one/two/").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("source").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                ],
                expected_output: Ok(into_anchored_system_path_vec(vec![
                    "target", "source", "one", "one/two",
                ])),
            },
        ];

        for is_compressed in [true, false] {
            for test in &tests {
                debug!("test: {}", test.name);
                let input_dir = tempdir()?;
                let archive_path = generate_tar(&input_dir, &test.input_files)?;
                let output_dir = tempdir()?;
                let output_dir_path = output_dir.path().to_string_lossy();
                let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

                let archive_path = if is_compressed {
                    compress_tar(&archive_path)?
                } else {
                    archive_path
                };

                let mut cache_reader = CacheReader::open(&archive_path)?;

                match (
                    cache_reader.restore(anchor, None).map(|(f, _)| f),
                    &test.expected_output,
                ) {
                    (Ok(restored_files), Err(expected_error)) => {
                        panic!("expected error: {expected_error:?}, received {restored_files:?}");
                    }
                    (Ok(restored_files), Ok(expected_files)) => {
                        assert_eq!(&restored_files, expected_files);
                    }
                    (Err(err), Err(expected_error)) => {
                        assert_eq!(&err.to_string(), expected_error);
                        continue;
                    }
                    (Err(err), Ok(_)) => {
                        panic!("unexpected error: {err:?}");
                    }
                };

                let expected_files = &test.expected_files;

                for expected_file in expected_files {
                    assert_file_exists(anchor, expected_file)?;
                }
            }
        }

        Ok(())
    }

    #[test_case(Path::new("source").try_into()?, Path::new("target"), "/Users/test/target", "C:\\Users\\test\\target" ; "hello world")]
    #[test_case(Path::new("child/source").try_into()?, Path::new("../sibling/target"), "/Users/test/sibling/target", "C:\\Users\\test\\sibling\\target" ; "Unix path subdirectory traversal")]
    #[test_case(Path::new("child/source").try_into()?, Path::new("..\\sibling\\target"), "/Users/test/child/..\\sibling\\target", "C:\\Users\\test\\sibling\\target" ; "Windows path subdirectory traversal")]
    fn test_canonicalize_linkname(
        processed_name: AnchoredSystemPathBuf,
        linkname: &Path,
        #[allow(unused_variables)] canonical_unix: &'static str,
        #[allow(unused_variables)] canonical_windows: &'static str,
    ) -> Result<()> {
        #[cfg(unix)]
        let anchor = AbsoluteSystemPath::new("/Users/test").unwrap();
        #[cfg(windows)]
        let anchor = AbsoluteSystemPath::new("C:\\Users\\test").unwrap();

        let received_path = canonicalize_linkname(anchor, &processed_name, linkname)?;

        #[cfg(unix)]
        assert_eq!(received_path.to_string(), canonical_unix);
        #[cfg(windows)]
        assert_eq!(received_path.to_string(), canonical_windows);

        Ok(())
    }

    mod absolute_path_tests {
        use super::*;

        #[test]
        fn test_absolute_path_file_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("/etc/passwd", b"malicious content");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_err(),
                "absolute path /etc/passwd should be rejected"
            );
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("malformed") || err.contains("Invalid"),
                "error should indicate malformed path, got: {err}"
            );

            Ok(())
        }

        #[test]
        fn test_absolute_path_directory_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("/tmp/evil", b"");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_err(),
                "absolute directory path should be rejected"
            );

            Ok(())
        }

        #[test]
        fn test_root_path_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("/", b"root");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "root path should be rejected");

            Ok(())
        }

        #[test]
        fn test_deep_absolute_path_rejected() -> Result<()> {
            let tar_bytes =
                generate_raw_tar_with_unsafe_path("/usr/local/bin/evil", b"#!/bin/sh\necho pwned");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "deep absolute path should be rejected");

            Ok(())
        }

        #[test]
        fn test_absolute_path_does_not_write_to_filesystem() -> Result<()> {
            let target = tempdir()?;
            let evil_file = target.path().join("should_not_exist");

            let tar_bytes =
                generate_raw_tar_with_unsafe_path(&evil_file.to_string_lossy(), b"evil");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let _ = reader.restore(anchor, None).map(|(f, _)| f);

            assert!(
                !evil_file.exists(),
                "file should not have been written to the absolute path"
            );

            Ok(())
        }
    }

    mod traversal_tests {
        use super::*;

        #[test]
        fn test_dot_dot_at_start_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("../escape", b"escaped");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "../escape should be rejected");

            Ok(())
        }

        #[test]
        fn test_dot_dot_in_middle_rejected() -> Result<()> {
            let tar_bytes =
                generate_raw_tar_with_unsafe_path("foo/../../../etc/passwd", b"escaped");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_err(),
                "path with excessive ../ components should be rejected"
            );

            Ok(())
        }

        #[test]
        fn test_current_dir_prefix_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("./../escape", b"escaped");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "./../escape should be rejected");

            Ok(())
        }

        #[test]
        fn test_dot_only_path_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path(".", b"dot");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "'.' path should be rejected");

            Ok(())
        }

        #[test]
        fn test_dot_dot_only_path_rejected() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("..", b"dotdot");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "'..' path should be rejected");

            Ok(())
        }
    }

    mod unicode_tests {
        use super::*;

        #[test]
        fn test_unicode_dot_lookalike_in_path() -> Result<()> {
            // U+FF0E is a fullwidth period that could be confused with '.'
            // If normalization occurs, "．．/escape" could become "../escape"
            let tar_bytes = generate_raw_tar(&[RawTarEntry::File {
                path: "\u{FF0E}\u{FF0E}/escape",
                body: b"escaped".to_vec(),
            }]);

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            // Fullwidth dots are not ASCII dots so this won't form a real
            // traversal. The key assertion is no file escapes the anchor.
            if result.is_ok() {
                let parent_escape = output_dir.path().parent().unwrap().join("escape");
                assert!(
                    !parent_escape.exists(),
                    "unicode lookalike should not escape the anchor"
                );
            }

            Ok(())
        }

        #[test]
        fn test_unicode_slash_lookalike_in_path() -> Result<()> {
            // U+2215 is "DIVISION SLASH" — could be confused with path separator.
            // On Windows the tar builder interprets backslashes as path separators
            // and rejects `..` components, so we use the raw tar generator.
            let tar_bytes =
                generate_raw_tar_with_unsafe_path("foo\u{2215}..\\..\\escape", b"escaped");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            if result.is_ok() {
                let parent_escape = output_dir.path().parent().unwrap().join("escape");
                assert!(
                    !parent_escape.exists(),
                    "unicode slash lookalike should not cause directory escape"
                );
            }

            Ok(())
        }

        #[test]
        fn test_nfc_vs_nfd_normalization() -> Result<()> {
            // e-acute as NFC (U+00E9) vs NFD (e + combining acute U+0301)
            // Two entries that look the same but differ in normalization could collide
            let tar_bytes = generate_raw_tar(&[
                RawTarEntry::Directory {
                    path: "caf\u{00E9}",
                },
                RawTarEntry::File {
                    path: "caf\u{00E9}/file_nfc",
                    body: b"nfc".to_vec(),
                },
                RawTarEntry::File {
                    path: "cafe\u{0301}/file_nfd",
                    body: b"nfd".to_vec(),
                },
            ]);

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // On macOS (HFS+), NFC and NFD paths resolve to the same file.
            // On Linux (ext4), they are distinct. Either behavior is acceptable
            // as long as we don't crash or escape the anchor.
            let result = reader.restore(anchor, None).map(|(f, _)| f);
            if let Err(e) = &result {
                let err_str = e.to_string();
                assert!(
                    !err_str.contains("outside of directory"),
                    "unicode normalization should not cause directory escape, got: {err_str}"
                );
            }

            Ok(())
        }

        #[test]
        fn test_null_byte_in_path() -> Result<()> {
            // Null bytes can truncate paths in C-based systems. The tar crate
            // should handle this, but we verify restore doesn't do something
            // unexpected.
            let mut buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut buf);
                let mut header = Header::new_gnu();
                header.set_size(5);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                // The tar crate validates path names, so this may fail at
                // the builder level — that's fine, it means the attack vector
                // is blocked upstream.
                let path_result =
                    builder.append_data(&mut header, "safe\x00/../../../etc/passwd", &b"evil"[..]);
                if path_result.is_err() {
                    return Ok(());
                }
                builder.into_inner().unwrap();
            }

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let _ = reader.restore(anchor, None).map(|(f, _)| f);

            // The only thing that matters is /etc/passwd wasn't overwritten
            assert!(
                !std::path::Path::new("/etc/passwd")
                    .metadata()
                    .map(|m| m.len() == 4)
                    .unwrap_or(false),
                "null byte injection should not write to arbitrary paths"
            );

            Ok(())
        }

        #[test]
        fn test_unicode_bidi_override_in_path() -> Result<()> {
            // Right-to-left override (U+202E) can visually disguise path names
            let tar_bytes = generate_raw_tar(&[RawTarEntry::File {
                path: "legit/\u{202E}dcba/file",
                body: b"bidi".to_vec(),
            }]);

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // We don't necessarily reject bidi characters, but must not escape
            let result = reader.restore(anchor, None).map(|(f, _)| f);
            if result.is_ok() {
                let parent = output_dir.path().parent().unwrap();
                for entry in fs::read_dir(parent)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    assert!(
                        !name.to_string_lossy().contains("dcba"),
                        "bidi override should not cause files to escape anchor"
                    );
                }
            }

            Ok(())
        }
    }

    mod long_path_tests {
        use super::*;

        #[test]
        fn test_path_exceeding_260_chars() -> Result<()> {
            // Windows MAX_PATH is 260 characters. Paths exceeding this could
            // cause unexpected behavior on some systems.
            let long_component = "a".repeat(250);
            let long_path: &'static str =
                Box::leak(format!("dir/{long_component}/file.txt").into_boxed_str());

            let tar_bytes = generate_raw_tar(&[
                RawTarEntry::Directory {
                    path: Box::leak("dir".to_string().into_boxed_str()),
                },
                RawTarEntry::Directory {
                    path: Box::leak(format!("dir/{long_component}").into_boxed_str()),
                },
                RawTarEntry::File {
                    path: long_path,
                    body: b"long path content".to_vec(),
                },
            ]);

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // On Unix this should succeed; on Windows it may fail due to MAX_PATH.
            // Either way, no panic or escape should occur.
            let _ = reader.restore(anchor, None).map(|(f, _)| f);

            Ok(())
        }

        #[test]
        fn test_deeply_nested_path() -> Result<()> {
            let mut components = Vec::new();
            for i in 0..50 {
                components.push(format!("d{i}"));
            }
            let deep_file = format!("{}/file.txt", components.join("/"));

            let mut entries: Vec<RawTarEntry> = Vec::new();
            let mut accumulated = String::new();
            for component in &components {
                if accumulated.is_empty() {
                    accumulated = component.clone();
                } else {
                    accumulated = format!("{accumulated}/{component}");
                }
                entries.push(RawTarEntry::Directory {
                    path: Box::leak(accumulated.clone().into_boxed_str()),
                });
            }
            entries.push(RawTarEntry::File {
                path: Box::leak(deep_file.into_boxed_str()),
                body: b"deep".to_vec(),
            });

            let tar_bytes = generate_raw_tar(&entries);

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_ok(),
                "deeply nested path should restore successfully"
            );

            Ok(())
        }

        #[test]
        fn test_total_path_length_over_4096() -> Result<()> {
            // PATH_MAX on Linux is typically 4096. Test paths near this limit.
            let component = "x".repeat(200);
            let path: &'static str = Box::leak(
                format!(
                    "{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/{c}/\
                     {c}/file",
                    c = component
                )
                .into_boxed_str(),
            );

            let mut entries = Vec::new();
            let mut accumulated = String::new();
            for _ in 0..20 {
                if accumulated.is_empty() {
                    accumulated = component.clone();
                } else {
                    accumulated = format!("{accumulated}/{component}");
                }
                entries.push(RawTarEntry::Directory {
                    path: Box::leak(accumulated.clone().into_boxed_str()),
                });
            }
            entries.push(RawTarEntry::File {
                path,
                body: b"very long".to_vec(),
            });

            let tar_bytes = generate_raw_tar(&entries);

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // May fail with IO error due to path length limits,
            // but should not panic or cause undefined behavior
            let _ = reader.restore(anchor, None).map(|(f, _)| f);

            Ok(())
        }
    }

    #[cfg(unix)]
    mod toctou_tests {
        use std::{
            sync::{Arc, Barrier},
            thread,
        };

        use super::*;

        #[test]
        fn test_concurrent_restore_to_same_anchor() -> Result<()> {
            let output_dir = tempdir()?;
            let anchor_path = output_dir.path().to_string_lossy().into_owned();

            let tar1 = generate_raw_tar(&[
                RawTarEntry::Directory { path: "shared" },
                RawTarEntry::File {
                    path: "shared/safe_file",
                    body: b"safe".to_vec(),
                },
            ]);

            let tar2 = generate_raw_tar(&[
                RawTarEntry::Directory { path: "shared" },
                RawTarEntry::File {
                    path: "shared/another_file",
                    body: b"also safe".to_vec(),
                },
            ]);

            let barrier = Arc::new(Barrier::new(2));
            let anchor1 = anchor_path.clone();
            let anchor2 = anchor_path.clone();
            let b1 = barrier.clone();
            let b2 = barrier.clone();

            let h1 = thread::spawn(move || {
                b1.wait();
                let mut reader = CacheReader::from_reader(&tar1[..], false).unwrap();
                let anchor = AbsoluteSystemPath::new(&anchor1).unwrap();
                reader.restore(anchor, None).map(|(f, _)| f)
            });

            let h2 = thread::spawn(move || {
                b2.wait();
                let mut reader = CacheReader::from_reader(&tar2[..], false).unwrap();
                let anchor = AbsoluteSystemPath::new(&anchor2).unwrap();
                reader.restore(anchor, None).map(|(f, _)| f)
            });

            let r1 = h1.join().expect("thread 1 panicked");
            let r2 = h2.join().expect("thread 2 panicked");

            let parent = output_dir.path().parent().unwrap();
            assert!(
                !parent.join("safe_file").exists(),
                "concurrent restore should not write outside anchor"
            );
            assert!(
                !parent.join("another_file").exists(),
                "concurrent restore should not write outside anchor"
            );

            assert!(
                r1.is_ok() || r2.is_ok(),
                "at least one concurrent restore should succeed"
            );

            Ok(())
        }

        #[test]
        fn test_concurrent_restore_with_symlink_attack() -> Result<()> {
            // One "archive" tries to create a symlink escape while
            // another writes through the same path name.
            let output_dir = tempdir()?;
            let anchor_path = output_dir.path().to_string_lossy().into_owned();

            let attacker_tar = generate_raw_tar(&[RawTarEntry::Symlink {
                link_path: "escape",
                link_target: "..",
            }]);

            let victim_tar = generate_raw_tar(&[
                RawTarEntry::Directory { path: "escape" },
                RawTarEntry::File {
                    path: "escape/payload",
                    body: b"should not escape".to_vec(),
                },
            ]);

            let barrier = Arc::new(Barrier::new(2));
            let anchor1 = anchor_path.clone();
            let anchor2 = anchor_path.clone();
            let b1 = barrier.clone();
            let b2 = barrier.clone();

            let h1 = thread::spawn(move || {
                b1.wait();
                let mut reader = CacheReader::from_reader(&attacker_tar[..], false).unwrap();
                let anchor = AbsoluteSystemPath::new(&anchor1).unwrap();
                let _ = reader.restore(anchor, None).map(|(f, _)| f);
            });

            let h2 = thread::spawn(move || {
                b2.wait();
                let mut reader = CacheReader::from_reader(&victim_tar[..], false).unwrap();
                let anchor = AbsoluteSystemPath::new(&anchor2).unwrap();
                let _ = reader.restore(anchor, None).map(|(f, _)| f);
            });

            h1.join().expect("attacker thread panicked");
            h2.join().expect("victim thread panicked");

            let parent = output_dir.path().parent().unwrap();
            assert!(
                !parent.join("payload").exists(),
                "TOCTOU attack should not allow writing outside anchor"
            );

            Ok(())
        }

        #[test]
        fn test_many_concurrent_restores() -> Result<()> {
            let output_dir = tempdir()?;
            let num_threads = 10;
            let barrier = Arc::new(Barrier::new(num_threads));
            let mut handles = Vec::new();

            for i in 0..num_threads {
                let b = barrier.clone();
                let base = output_dir.path().to_path_buf();

                handles.push(thread::spawn(move || {
                    let subdir = base.join(format!("worker_{i}"));
                    std::fs::create_dir_all(&subdir).unwrap();
                    let anchor_str = subdir.to_string_lossy().into_owned();

                    let dir_path: &'static str = Box::leak(format!("output_{i}").into_boxed_str());
                    let file_path: &'static str =
                        Box::leak(format!("output_{i}/result.txt").into_boxed_str());

                    let tar = generate_raw_tar(&[
                        RawTarEntry::Directory { path: dir_path },
                        RawTarEntry::File {
                            path: file_path,
                            body: format!("result from thread {i}").into_bytes(),
                        },
                    ]);

                    b.wait();
                    let mut reader = CacheReader::from_reader(&tar[..], false).unwrap();
                    let anchor = AbsoluteSystemPath::new(&anchor_str).unwrap();
                    reader.restore(anchor, None).map(|(f, _)| f)
                }));
            }

            for (i, handle) in handles.into_iter().enumerate() {
                let result = handle.join().expect("thread panicked");
                assert!(
                    result.is_ok(),
                    "worker {i} failed: {:?}",
                    result.unwrap_err()
                );
            }

            Ok(())
        }
    }

    mod malformed_tar_tests {
        use super::*;

        #[test]
        fn test_empty_path_in_entry() -> Result<()> {
            let mut buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut buf);
                let mut header = Header::new_gnu();
                header.set_size(4);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                let result = builder.append_data(&mut header, "", &b"test"[..]);
                if result.is_err() {
                    return Ok(());
                }
                builder.into_inner().unwrap();
            }

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "empty path should be rejected");

            Ok(())
        }

        #[test]
        fn test_double_slash_in_path() -> Result<()> {
            let tar_bytes = generate_raw_tar_with_unsafe_path("foo//bar", b"content");

            let mut reader = CacheReader::from_reader(&tar_bytes[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "double slash in path should be rejected");

            Ok(())
        }

        #[test]
        fn test_symlink_with_empty_target() -> Result<()> {
            let mut buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut buf);
                let mut header = Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                let result = builder.append_data(&mut header, "orphan_link", empty());
                if result.is_err() {
                    return Ok(());
                }
                builder.into_inner().unwrap();
            }

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "symlink without target should be rejected");

            Ok(())
        }

        #[test]
        fn test_hardlink_entry_type_rejected() -> Result<()> {
            let mut buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut buf);
                let mut header = Header::new_gnu();
                header.set_entry_type(tar::EntryType::Link);
                header.set_size(0);
                builder
                    .append_link(&mut header, "hardlink", "target")
                    .unwrap();
                builder.into_inner().unwrap();
            }

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "hardlink entry type should be rejected");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("unsupported file type"),
                "should report unsupported file type, got: {err}"
            );

            Ok(())
        }

        #[test]
        fn test_character_device_entry_type_rejected() -> Result<()> {
            let mut buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut buf);
                let mut header = Header::new_gnu();
                header.set_entry_type(tar::EntryType::Char);
                header.set_size(0);
                header.set_mode(0o644);
                builder
                    .append_data(&mut header, "chardev", empty())
                    .unwrap();
                builder.into_inner().unwrap();
            }

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_err(),
                "character device entry type should be rejected"
            );

            Ok(())
        }

        #[test]
        fn test_block_device_entry_type_rejected() -> Result<()> {
            let mut buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut buf);
                let mut header = Header::new_gnu();
                header.set_entry_type(tar::EntryType::Block);
                header.set_size(0);
                header.set_mode(0o644);
                builder
                    .append_data(&mut header, "blockdev", empty())
                    .unwrap();
                builder.into_inner().unwrap();
            }

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_err(),
                "block device entry type should be rejected"
            );

            Ok(())
        }

        #[test]
        fn test_completely_invalid_tar_data() -> Result<()> {
            let garbage = b"this is not a tar file at all, just random garbage data";

            let mut reader = CacheReader::from_reader(&garbage[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // Should not panic
            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(
                result.is_err() || result.unwrap().is_empty(),
                "garbage data should produce error or empty result"
            );

            Ok(())
        }

        #[test]
        fn test_truncated_tar_data() -> Result<()> {
            let tar_bytes = generate_raw_tar(&[RawTarEntry::File {
                path: "legitimate_file",
                body: vec![0u8; 10000],
            }]);
            let truncated = &tar_bytes[..tar_bytes.len() / 2];

            let mut reader = CacheReader::from_reader(truncated, false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "truncated tar should produce an error");

            Ok(())
        }

        #[test]
        fn test_mixed_valid_and_malicious_entries() -> Result<()> {
            // Build a tar with a valid file followed by a traversal entry.
            // We need to construct this manually since the tar builder rejects
            // `..` in paths.
            use std::io::Write;

            let mut buf = Vec::new();

            // First entry: safe_file (valid, uses the builder)
            {
                let mut inner_buf = Vec::new();
                let mut builder = tar::Builder::new(&mut inner_buf);
                let mut header = Header::new_gnu();
                header.set_size(12);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                builder
                    .append_data(&mut header, "safe_file", &b"safe content"[..])
                    .unwrap();
                builder.into_inner().unwrap();
                // Strip the 1024-byte end-of-archive marker
                buf.write_all(&inner_buf[..inner_buf.len() - 1024]).unwrap();
            }

            // Second entry: ../escaped_file (malicious, written raw)
            {
                let path = "../escaped_file";
                let body = b"malicious";
                let mut header_bytes = [0u8; 512];
                let name_bytes = path.as_bytes();
                header_bytes[..name_bytes.len()].copy_from_slice(name_bytes);
                header_bytes[100..107].copy_from_slice(b"0000644");
                header_bytes[108..115].copy_from_slice(b"0001000");
                header_bytes[116..123].copy_from_slice(b"0001000");
                let size_str = format!("{:011o}", body.len());
                header_bytes[124..135].copy_from_slice(size_str.as_bytes());
                header_bytes[136..147].copy_from_slice(b"00000000000");
                header_bytes[156] = b'0';
                header_bytes[257..263].copy_from_slice(b"ustar\0");
                header_bytes[263..265].copy_from_slice(b"00");
                header_bytes[148..156].copy_from_slice(b"        ");
                let checksum: u32 = header_bytes.iter().map(|&b| b as u32).sum();
                let checksum_str = format!("{:06o}\0 ", checksum);
                header_bytes[148..156].copy_from_slice(checksum_str.as_bytes());
                buf.write_all(&header_bytes).unwrap();
                buf.write_all(body).unwrap();
                let padding = 512 - (body.len() % 512);
                if padding < 512 {
                    buf.write_all(&vec![0u8; padding]).unwrap();
                }
            }

            // End-of-archive
            buf.write_all(&[0u8; 1024]).unwrap();

            let mut reader = CacheReader::from_reader(&buf[..], false)?;
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy().into_owned();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let result = reader.restore(anchor, None).map(|(f, _)| f);
            assert!(result.is_err(), "archive with malicious entry should fail");

            let parent = output_dir.path().parent().unwrap();
            assert!(
                !parent.join("escaped_file").exists(),
                "malicious file should not be written outside anchor"
            );

            Ok(())
        }
    }

    // Cross-platform regression test for #8476. Uses turbopath's
    // symlink_to_dir which works on both Unix and Windows.
    #[test]
    fn test_pre_existing_symlink_replaced_cross_platform() -> Result<()> {
        let input_dir = tempdir()?;
        let archive_path = generate_tar(
            &input_dir,
            &[
                TarFile::Directory {
                    path: AnchoredSystemPathBuf::from_raw("dist").unwrap(),
                },
                TarFile::File {
                    path: AnchoredSystemPathBuf::from_raw("dist/index.js").unwrap(),
                    body: b"console.log('hello')".to_vec(),
                },
            ],
        )?;

        let output_dir = tempdir()?;
        let output_dir_path = output_dir.path().to_string_lossy();
        let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

        let output_src = anchor.join_component("src");
        output_src.create_dir_all()?;
        let output_dist = anchor.join_component("dist");
        output_dist.symlink_to_dir("src")?;

        let mut cache_reader = CacheReader::open(&archive_path)?;
        cache_reader.restore(anchor, None)?;

        let dist_meta = output_dist.symlink_metadata()?;
        assert!(
            !dist_meta.is_symlink(),
            "dist should be a real directory, not a symlink"
        );
        assert!(dist_meta.is_dir());

        let content = fs::read(output_dist.join_component("index.js").as_path())?;
        assert_eq!(content, b"console.log('hello')");

        assert!(
            !output_src.join_component("index.js").try_exists()?,
            "file must not leak through symlink into src/"
        );

        Ok(())
    }

    // Regression tests for https://github.com/vercel/turborepo/issues/8476
    //
    // When a symlink exists on disk at a path where the tar expects a real
    // directory, the restore must replace the symlink with a directory so
    // files land at their literal paths instead of leaking through the
    // symlink to the wrong location.
    #[cfg(unix)]
    mod pre_existing_symlink_tests {
        use super::*;

        #[test]
        fn test_pre_existing_symlink_does_not_overwrite_target() -> Result<()> {
            let input_dir = tempdir()?;
            let archive_path = generate_tar(
                &input_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("dist/index.js").unwrap(),
                        body: b"console.log('hello')".to_vec(),
                    },
                ],
            )?;

            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let output_src = anchor.join_component("src");
            output_src.create_dir_all()?;
            let output_dist = anchor.join_component("dist");
            std::os::unix::fs::symlink("src", output_dist.as_path())?;

            assert!(
                !output_src.join_component("index.js").try_exists()?,
                "src is empty before restore"
            );

            let mut cache_reader = CacheReader::open(&archive_path)?;
            let (restored, _) = cache_reader.restore(anchor, None)?;

            assert_eq!(
                restored,
                into_anchored_system_path_vec(vec!["dist", "dist/index.js"])
            );

            let dist_meta = output_dist.symlink_metadata()?;
            assert!(
                !dist_meta.is_symlink(),
                "dist should be a real directory, not a symlink"
            );
            assert!(dist_meta.is_dir());

            let content = fs::read(output_dist.join_component("index.js").as_path())?;
            assert_eq!(content, b"console.log('hello')");

            assert!(
                !output_src.join_component("index.js").try_exists()?,
                "file must not leak through symlink into src/"
            );

            Ok(())
        }

        #[test]
        fn test_pre_existing_nested_symlink_replaced() -> Result<()> {
            let input_dir = tempdir()?;
            let archive_path = generate_tar(
                &input_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/runtime/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("dist/runtime/plugin.js").unwrap(),
                        body: b"plugin".to_vec(),
                    },
                ],
            )?;

            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let src_runtime = anchor.join_component("src").join_component("runtime");
            src_runtime.create_dir_all()?;
            let dist = anchor.join_component("dist");
            dist.create_dir_all()?;
            std::os::unix::fs::symlink("../src/runtime", dist.join_component("runtime").as_path())?;

            let mut cache_reader = CacheReader::open(&archive_path)?;
            let (restored, _) = cache_reader.restore(anchor, None)?;

            assert_eq!(
                restored,
                into_anchored_system_path_vec(vec![
                    "dist",
                    "dist/runtime",
                    "dist/runtime/plugin.js"
                ])
            );

            let runtime_meta = dist.join_component("runtime").symlink_metadata()?;
            assert!(
                !runtime_meta.is_symlink(),
                "dist/runtime should be a real directory"
            );
            assert!(runtime_meta.is_dir());

            let content = fs::read(
                dist.join_component("runtime")
                    .join_component("plugin.js")
                    .as_path(),
            )?;
            assert_eq!(content, b"plugin");

            assert!(
                !src_runtime
                    .join_component("plugin.js")
                    .try_exists()
                    .unwrap_or(false),
                "file must not leak through symlink into src/runtime/"
            );

            Ok(())
        }

        #[test]
        fn test_pre_existing_absolute_symlink_replaced() -> Result<()> {
            let input_dir = tempdir()?;
            let archive_path = generate_tar(
                &input_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/runtime/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("dist/runtime/plugin.js").unwrap(),
                        body: b"plugin".to_vec(),
                    },
                ],
            )?;

            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let src_runtime = anchor.join_component("src").join_component("runtime");
            src_runtime.create_dir_all()?;
            let dist = anchor.join_component("dist");
            dist.create_dir_all()?;
            // Use an absolute symlink target (matches the actual bug report)
            std::os::unix::fs::symlink(
                src_runtime.as_path(),
                dist.join_component("runtime").as_path(),
            )?;

            let mut cache_reader = CacheReader::open(&archive_path)?;
            cache_reader.restore(anchor, None)?;

            let runtime_meta = dist.join_component("runtime").symlink_metadata()?;
            assert!(
                !runtime_meta.is_symlink(),
                "absolute symlink should be replaced"
            );

            let content = fs::read(
                dist.join_component("runtime")
                    .join_component("plugin.js")
                    .as_path(),
            )?;
            assert_eq!(content, b"plugin");

            assert!(
                !src_runtime
                    .join_component("plugin.js")
                    .try_exists()
                    .unwrap_or(false),
                "file must not leak through absolute symlink"
            );

            Ok(())
        }

        #[test]
        fn test_pre_existing_intermediate_symlink_replaced() -> Result<()> {
            let input_dir = tempdir()?;
            // No explicit directory entries — only the file. This exercises
            // the safe_mkdir_file -> safe_mkdir_all path.
            let archive_path = generate_tar(
                &input_dir,
                &[TarFile::File {
                    path: AnchoredSystemPathBuf::from_raw("a/b/c/file.txt").unwrap(),
                    body: b"content".to_vec(),
                }],
            )?;

            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            let real_b = anchor.join_component("real_b");
            real_b.create_dir_all()?;
            let a = anchor.join_component("a");
            a.create_dir_all()?;
            // Intermediate component "b" is a symlink
            std::os::unix::fs::symlink("../real_b", a.join_component("b").as_path())?;

            let mut cache_reader = CacheReader::open(&archive_path)?;
            cache_reader.restore(anchor, None)?;

            let b_meta = a.join_component("b").symlink_metadata()?;
            assert!(
                !b_meta.is_symlink(),
                "intermediate symlink a/b should be replaced"
            );
            assert!(b_meta.is_dir());

            let content = fs::read(
                a.join_component("b")
                    .join_component("c")
                    .join_component("file.txt")
                    .as_path(),
            )?;
            assert_eq!(content, b"content");

            assert!(
                !real_b
                    .join_component("c")
                    .join_component("file.txt")
                    .try_exists()
                    .unwrap_or(false),
                "file must not leak through intermediate symlink"
            );

            Ok(())
        }

        #[test]
        fn test_sequential_restores_symlink_then_directory() -> Result<()> {
            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // The symlink target must exist for the first restore
            let src_runtime = anchor.join_component("src").join_component("runtime");
            src_runtime.create_dir_all()?;

            // First tar: simulates dev:prepare creating a symlink
            let tar1_dir = tempdir()?;
            let tar1_path = generate_tar(
                &tar1_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("dist/runtime").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("../src/runtime").unwrap(),
                    },
                ],
            )?;

            let mut reader1 = CacheReader::open(&tar1_path)?;
            reader1.restore(anchor, None)?;

            let runtime_after_first = anchor
                .join_component("dist")
                .join_component("runtime")
                .symlink_metadata()?;
            assert!(
                runtime_after_first.is_symlink(),
                "dist/runtime should be a symlink after first restore"
            );

            // Second tar: simulates build overriding with real files
            let tar2_dir = tempdir()?;
            let tar2_path = generate_tar(
                &tar2_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/runtime/").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("dist/runtime/plugin.js").unwrap(),
                        body: b"built output".to_vec(),
                    },
                ],
            )?;

            let mut reader2 = CacheReader::open(&tar2_path)?;
            reader2.restore(anchor, None)?;

            let runtime_after_second = anchor
                .join_component("dist")
                .join_component("runtime")
                .symlink_metadata()?;
            assert!(
                !runtime_after_second.is_symlink(),
                "dist/runtime should be a real directory after second restore"
            );
            assert!(runtime_after_second.is_dir());

            let content = fs::read(
                anchor
                    .join_component("dist")
                    .join_component("runtime")
                    .join_component("plugin.js")
                    .as_path(),
            )?;
            assert_eq!(content, b"built output");

            assert!(
                !src_runtime
                    .join_component("plugin.js")
                    .try_exists()
                    .unwrap_or(false),
                "file must not leak through symlink from first restore"
            );

            Ok(())
        }

        #[test]
        fn test_pre_existing_symlink_outside_anchor_does_not_escape() -> Result<()> {
            let input_dir = tempdir()?;
            let archive_path = generate_tar(
                &input_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist").unwrap(),
                    },
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("dist/sub").unwrap(),
                    },
                    TarFile::File {
                        path: AnchoredSystemPathBuf::from_raw("dist/sub/file.txt").unwrap(),
                        body: b"safe content".to_vec(),
                    },
                ],
            )?;

            let outside_dir = tempdir()?;
            let outside_dir_path = outside_dir.path().to_string_lossy();

            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // Pre-existing symlink pointing OUTSIDE the anchor
            std::os::unix::fs::symlink(
                &*outside_dir_path,
                anchor.join_component("dist").as_path(),
            )?;

            let mut cache_reader = CacheReader::open(&archive_path)?;
            cache_reader.restore(anchor, None)?;

            let dist_meta = anchor.join_component("dist").symlink_metadata()?;
            assert!(
                !dist_meta.is_symlink(),
                "dist should be a real directory, not a symlink"
            );
            assert!(dist_meta.is_dir());

            let content = fs::read(
                anchor
                    .join_component("dist")
                    .join_component("sub")
                    .join_component("file.txt")
                    .as_path(),
            )?;
            assert_eq!(content, b"safe content");

            assert!(
                !outside_dir.path().join("sub").exists(),
                "files must not leak outside the anchor"
            );

            Ok(())
        }

        #[test]
        fn test_tar_symlink_not_replaced_by_same_restore() -> Result<()> {
            let input_dir = tempdir()?;
            let archive_path = generate_tar(
                &input_dir,
                &[
                    TarFile::Directory {
                        path: AnchoredSystemPathBuf::from_raw("target/").unwrap(),
                    },
                    TarFile::Symlink {
                        link_path: AnchoredSystemPathBuf::from_raw("link").unwrap(),
                        link_target: AnchoredSystemPathBuf::from_raw("target").unwrap(),
                    },
                ],
            )?;

            let output_dir = tempdir()?;
            let output_dir_path = output_dir.path().to_string_lossy();
            let anchor = AbsoluteSystemPath::new(&output_dir_path)?;

            // Pre-existing symlink at the same path (should be clobbered by
            // the tar's own symlink, not converted to a directory)
            let target = anchor.join_component("target");
            target.create_dir_all()?;
            let link = anchor.join_component("link");
            std::os::unix::fs::symlink("target", link.as_path())?;

            let mut cache_reader = CacheReader::open(&archive_path)?;
            cache_reader.restore(anchor, None)?;

            let link_meta = link.symlink_metadata()?;
            assert!(
                link_meta.is_symlink(),
                "symlink from the tar itself should be preserved"
            );
            let actual_target = fs::read_link(link.as_path())?;
            assert_eq!(
                actual_target.to_str().unwrap(),
                "target",
                "symlink target should match tar entry"
            );

            Ok(())
        }
    }
}
