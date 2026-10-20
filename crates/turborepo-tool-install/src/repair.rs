//! Sealed resolution evidence for repair, never healthy resources or execution.
use super::*;

impl Store {
    /// Capture a recorded selection even when its resources are
    /// missing/damaged. Invalid metadata is never absence. Capture the
    /// independent record, or an intact legacy generation record. Missing both
    /// fails closed; checks retain the exact captured record path.
    pub fn repair_generation(&self) -> Result<GenerationExpectation, Error> {
        self.check_root()?;
        let manifest = Self::manifest_bytes(&self.root)?;
        let inventory = Self::inventory_metadata(&self.root)?.ok_or(Error::InvalidInventory)?;
        let hash = inventory
            .record_sha256
            .as_ref()
            .ok_or(Error::InvalidInventory)?;
        let generation = self.root.join(&inventory.generation);
        let independent = self.root.join(format!("record-{hash}.json"));
        let path = match fs::symlink_metadata(&independent) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                real_directory(&generation)?;
                generation.join("record.json")
            }
            Err(e) => return Err(e.into()),
            Ok(_) => independent,
        };
        let record = Self::read_record(&path, &fs::symlink_metadata(&path)?, hash)?;
        // Existing corrupt record bytes are not repaired by silently ignoring them.
        match fs::symlink_metadata(generation.join("record.json")) {
            Ok(_) => {
                real_directory(&generation)?;
                if Self::record(&self.root, &inventory)?.as_ref() != Some(&record) {
                    return Err(Error::InvalidInventory);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut expected = GenerationExpectation {
            identity: self.identity.clone(),
            manifest,
            current: Some(Current {
                tools: inventory.tools.into_iter().map(|t| t.tool).collect(),
                bin: generation.join("bin"),
                record: Some(record),
            }),
            repair: Some((path.clone(), None)),
        };
        expected.repair = Some((path, self.repair_state(&expected)?));
        self.check_generation(&expected)?;
        Ok(expected)
    }

    pub(super) fn repair_state(
        &self,
        expected: &GenerationExpectation,
    ) -> Result<Option<String>, Error> {
        let current = expected.current.as_ref().ok_or(Error::InvalidInventory)?;
        let record = current.record.as_ref().ok_or(Error::InvalidInventory)?;
        let (path, _) = expected.repair.as_ref().ok_or(Error::InvalidInventory)?;
        if Self::read_record(path, &fs::symlink_metadata(path)?, &record.hash())? != *record {
            return Err(Error::InvalidInventory);
        }
        let generation = current.bin.parent().ok_or(Error::UnsafePath)?;
        match fs::symlink_metadata(generation) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
            Ok(_) => state_hash(generation, false).map(Some),
        }
    }

    // Immutable, bounded bookkeeping, selected ONLY by the atomic manifest hash.
    // Orphans are inactive; a crash before manifest replacement selects nothing.
    pub(super) fn preserve_record(&self, record: &Record) -> Result<(), Error> {
        let path = self.root.join(format!("record-{}.json", record.hash()));
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                Self::read_record(&path, &metadata, &record.hash())?;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut staged = tempfile::NamedTempFile::new_in(&self.root)?;
        staged.write_all(record.bytes())?;
        staged.as_file().sync_all()?;
        staged
            .persist_noclobber(path)
            .map_err(|e| Error::Io(e.error))?;
        sync_directory(&self.root)
    }
}
