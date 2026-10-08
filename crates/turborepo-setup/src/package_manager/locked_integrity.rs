//! Pure frozen-pin preflight. No storage, metadata resolution, or traffic.
use super::*;

impl Declaration {
    /// Select the version-applicable OR alternative using native onFail rules.
    /// Different applicable digest choices (including unpinned alternatives)
    /// are explicitly unsupported, never flattened into AND or silently
    /// ignored. SHA-256 is checked against the selected lock before any
    /// preparation; SHA-512 is retained for metadata/byte verification and
    /// reuse identity.
    pub fn locked_integrity(
        &self,
        release: &Version,
        locked_sha256: &str,
    ) -> Result<Option<CorepackIntegrity>, Error> {
        if !self.matches(release) {
            return Err(invalid(
                TOP,
                "locked version does not satisfy native declarations",
            ));
        }
        let top = self
            .package_manager
            .as_ref()
            .map(|r| checked(r, locked_sha256))
            .transpose()?
            .flatten();
        let mut applicable = self.dev_engines.iter().filter(|r| {
            r.manager == self.manager && r.request.as_ref().is_none_or(|v| v.matches(release))
        });
        let dev = match applicable.next() {
            None => None, // Only possible with an authoritative advisory override.
            Some(first) => {
                let pin = checked(first, locked_sha256)?;
                for alternative in applicable {
                    if checked(alternative, locked_sha256)? != pin {
                        return Err(invalid(
                            DEV,
                            "ambiguous applicable authored integrity alternatives",
                        ));
                    }
                }
                pin
            }
        };
        match (top, dev) {
            (Some(a), Some(b)) if a.algorithm == b.algorithm && a != b => Err(invalid(
                DEV,
                "conflicting authoritative and devEngines integrity",
            )),
            (Some(a), Some(b)) => Ok(Some(if b.algorithm == "sha512" { b } else { a })),
            (a, b) => Ok(a.or(b)),
        }
    }
}

fn checked(request: &Request, locked: &str) -> Result<Option<CorepackIntegrity>, Error> {
    let Some(pin) = &request.integrity else {
        return Ok(None);
    };
    let length = match pin.algorithm {
        "sha256" => 64,
        "sha512" => 128,
        _ => {
            return Err(invalid(
                &request.source,
                "unsupported authored integrity algorithm",
            ));
        }
    };
    if pin.digest.len() != length || !pin.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(&request.source, "invalid authored integrity"));
    }
    let pin = CorepackIntegrity {
        algorithm: pin.algorithm,
        digest: pin.digest.to_ascii_lowercase(),
    };
    if pin.algorithm == "sha256" && pin.digest != locked {
        return Err(invalid(
            &request.source,
            "authored SHA-256 contradicts locked artifact",
        ));
    }
    Ok(Some(pin))
}

#[cfg(test)]
mod tests;
