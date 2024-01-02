use std::collections::HashMap;

use super::identifiers::{Descriptor, Ident};

/// A data structure for resolving descriptors when the protocol isn't known
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DescriptorResolver {
    mapping: HashMap<Key, Entry>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    ident: Ident<'static>,
    range: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
struct Entry {
    without: Option<String>,
    with: Option<RangeAndProtocol>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
struct RangeAndProtocol {
    protocol: String,
    range: String,
}

impl DescriptorResolver {
    /// Add a descriptor to the resolver
    pub fn insert(&mut self, descriptor: &Descriptor) -> Option<String> {
        let key = Key::new(descriptor)?;
        let entry = self.mapping.entry(key).or_default();
        entry.insert_descriptor(descriptor)
    }

    /// If given a descriptor without a protocol it will return all matching
    /// descriptors with a protocol
    pub fn get(&self, descriptor: &Descriptor) -> Option<&str> {
        let key = Key::new(descriptor)?;
        self.mapping.get(&key).and_then(|e| e.get(descriptor))
    }
}

impl Key {
    fn new(desc: &Descriptor) -> Option<Self> {
        let ident = desc.ident.to_owned();
        let range = Descriptor::strip_protocol(desc.range()?).to_string();
        Some(Key { ident, range })
    }
}

impl Entry {
    // Insert the given descriptor's range into the correct slot depending if it is
    // with or without a protocol
    fn insert_descriptor(&mut self, descriptor: &Descriptor) -> Option<String> {
        let range = descriptor.range()?.to_string();
        match descriptor.protocol() {
            Some(protocol) => {
                // Yarn 4 made the default npm protocol explicit in the lockfile.
                // In order to return the more specific protocol we avoid overwriting other
                // protocols with the now explicit npm protocol.
                if protocol != "npm" || self.with.is_none() {
                    match self.with.replace(RangeAndProtocol {
                        range,
                        protocol: protocol.to_string(),
                    }) {
                        // We only return an ejected range if the protocol isn't the default npm
                        // protocol
                        Some(RangeAndProtocol { range, protocol }) if protocol != "npm" => {
                            Some(range)
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            None => self.without.replace(range),
        }
    }

    fn get(&self, descriptor: &Descriptor) -> Option<&str> {
        // We only return the without protocol range if `without` is present
        // and the given descriptor is also without a protocol
        if self.without.is_some() && descriptor.protocol().is_none() {
            self.without.as_deref()
        } else {
            self.with.as_ref().map(|x| x.range.as_str())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_descriptor_reconstruction() {
        let mut resolver = DescriptorResolver::default();
        let babel_npm = Descriptor::new("@babel/core", "npm:^5.0.0").unwrap();
        let babel_file = Descriptor::new("@babel/core", "file:4.5.0").unwrap();
        assert!(resolver.insert(&babel_npm).is_none());
        assert!(resolver.insert(&babel_file).is_none());
        let babel_5 = Descriptor::new("@babel/core", "^5.0.0").unwrap();
        let babel_4 = Descriptor::new("@babel/core", "4.5.0").unwrap();
        assert_eq!(resolver.get(&babel_5), Some("npm:^5.0.0"));
        assert_eq!(resolver.get(&babel_4), Some("file:4.5.0"));
    }

    #[test]
    fn test_descriptors_without_protocols() {
        let mut resolver = DescriptorResolver::default();
        let workspace = Descriptor::new("internal-workspace", "*").unwrap();
        let workspace_with_protocol = Descriptor::new("internal-workspace", "workspace:*").unwrap();
        assert!(resolver.insert(&workspace).is_none());
        assert!(resolver.insert(&workspace_with_protocol).is_none());
        assert_eq!(resolver.get(&workspace), Some("*"));
        assert_eq!(resolver.get(&workspace_with_protocol), Some("workspace:*"));
    }
}
