use std::collections::HashMap;

use super::identifiers::{Descriptor, Ident};

/// A data structure for resolving descriptors when the protocol isn't known
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DescriptorResolver<'a> {
    mapping: HashMap<Key<'a>, Entry<'a>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct Key<'a> {
    ident: Ident<'a>,
    range: &'a str,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
struct Entry<'a> {
    without: Option<&'a str>,
    with: Option<&'a str>,
}

impl<'a> DescriptorResolver<'a> {
    /// Add a descriptor to the resolver
    pub fn insert(&mut self, descriptor: &Descriptor<'a>) -> Option<&'a str> {
        let key = Key::new(descriptor)?;
        let entry = self.mapping.entry(key).or_default();
        entry.insert_descriptor(descriptor)
    }

    /// If given a descriptor without a protocol it will return all matching
    /// descriptors with a protocol
    pub fn get(&self, descriptor: &Descriptor) -> Option<&'a str> {
        let key = Key::new(descriptor)?;
        self.mapping.get(&key).and_then(|e| e.get(descriptor))
    }
}

impl<'a> Key<'a> {
    fn new(desc: &Descriptor<'a>) -> Option<Self> {
        let ident = desc.ident.clone();
        let range = Descriptor::strip_protocol(desc.range()?);
        Some(Key { ident, range })
    }
}

impl<'a> Entry<'a> {
    // Insert the given descriptor's range into the correct slot depending if it is
    // with or without a protocol
    fn insert_descriptor(&mut self, descriptor: &Descriptor<'a>) -> Option<&'a str> {
        let range = descriptor.range()?;
        match descriptor.protocol().is_some() {
            true => self.with.replace(range),
            false => self.without.replace(range),
        }
    }

    fn get(&self, descriptor: &Descriptor) -> Option<&'a str> {
        // We only return the without protocol range if `without` is present
        // and the given descriptor is also without a protocol
        if self.without.is_some() && descriptor.protocol().is_none() {
            self.without
        } else {
            self.with
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
