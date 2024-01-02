use std::{backtrace::Backtrace, collections::HashMap, io::Read};

use petgraph::graph::DiGraph;
use sha2::{Digest, Sha512};
use tar::Entry;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    cache_archive::{
        restore_directory::{restore_directory, CachedDirTree},
        restore_regular::restore_regular,
        restore_symlink::{
            canonicalize_linkname, restore_symlink, restore_symlink_allow_missing_target,
        },
    },
    CacheError,
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
    ) -> Result<Vec<AnchoredSystemPathBuf>, CacheError> {
        let mut restored = Vec::new();
        anchor.create_dir_all()?;

        // We're going to make the following two assumptions here for "fast"
        // path restoration:
        // - All directories are enumerated in the `tar`.
        // - The contents of the tar are enumerated depth-first.
        //
        // This allows us to avoid:
        // - Attempts at recursive creation of directories.
        // - Repetitive `lstat` on restore of a file.
        //
        // Violating these assumptions won't cause things to break but we're
        // only going to maintain an `lstat` cache for the current tree.
        // If you violate these assumptions and the current cache does
        // not apply for your path, it will clobber and re-start from the common
        // shared prefix.
        let dir_cache = CachedDirTree::new(anchor.to_owned());
        let mut tr = tar::Archive::new(&mut self.reader);

        Self::restore_entries(&mut tr, &mut restored, dir_cache, anchor)?;
        Ok(restored)
    }

    fn restore_entries<T: Read>(
        tr: &mut tar::Archive<T>,
        restored: &mut Vec<AnchoredSystemPathBuf>,
        mut dir_cache: CachedDirTree,
        anchor: &AbsoluteSystemPath,
    ) -> Result<(), CacheError> {
        // On first attempt to restore it's possible that a link target doesn't exist.
        // Save them and topologically sort them.
        let mut symlinks = Vec::new();

        for entry in tr.entries()? {
            let mut entry = entry?;
            match restore_entry(&mut dir_cache, anchor, &mut entry) {
                Err(CacheError::LinkTargetDoesNotExist(_, _)) => {
                    symlinks.push(entry);
                }
                Err(e) => return Err(e),
                Ok(restored_path) => restored.push(restored_path),
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
            let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.header().path()?)?;
            let processed_sourcename =
                canonicalize_linkname(anchor, &processed_name, processed_name.as_path())?;
            // symlink must have a linkname
            let linkname = entry
                .header()
                .link_name()?
                .expect("symlink without linkname");

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

fn restore_entry<T: Read>(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &mut Entry<T>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let header = entry.header();

    match header.entry_type() {
        tar::EntryType::Directory => restore_directory(dir_cache, anchor, entry.header()),
        tar::EntryType::Regular => restore_regular(dir_cache, anchor, entry),
        tar::EntryType::Symlink => restore_symlink(dir_cache, anchor, entry),
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
    use tempfile::{tempdir, TempDir};
    use test_case::test_case;
    use tracing::debug;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use crate::cache_archive::{restore::CacheReader, restore_symlink::canonicalize_linkname};

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

        let output_file_path = format!("{}.zst", archive_path);
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
            let result = cache_reader.restore(anchor);
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
            let result = cache_reader.restore(anchor);
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

                match (cache_reader.restore(anchor), &test.expected_output) {
                    (Ok(restored_files), Err(expected_error)) => {
                        panic!(
                            "expected error: {:?}, received {:?}",
                            expected_error, restored_files
                        );
                    }
                    (Ok(restored_files), Ok(expected_files)) => {
                        assert_eq!(&restored_files, expected_files);
                    }
                    (Err(err), Err(expected_error)) => {
                        assert_eq!(&err.to_string(), expected_error);
                        continue;
                    }
                    (Err(err), Ok(_)) => {
                        panic!("unexpected error: {:?}", err);
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
}
