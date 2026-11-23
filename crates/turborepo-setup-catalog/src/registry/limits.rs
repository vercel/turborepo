//! Limits on a registry document, checked before the recipe reader reads it.
//! The reader renders and evaluates text from the document, and some text
//! crashes it or runs without bound:
//!
//! - Its template lexer slices by character index, so a tag that is not ASCII
//!   can panic it, and it recurses once for every nested parenthesis.
//! - Planning tries a release name for every version prefix of a recipe, and
//!   resolution asks the network about each.
//!
//! The walk decodes the document with the reader's own YAML decoder and
//! options and visits every scalar, key or value, so no field, alias, escape,
//! or tag carries text past it that the reader would see. It refuses a tag
//! that YAML does not define, the one thing the reader decodes differently.

use std::{collections::BTreeSet, fmt};

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

use super::{Position, Problem};

const MAX_TEMPLATE_BYTES: usize = 1024;
const MAX_VERSION_PREFIXES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Limit {
    #[error("text with a template tag has a character that is not ASCII")]
    TemplateNotAscii,
    #[error("text with a template tag is longer than {MAX_TEMPLATE_BYTES} bytes")]
    TemplateTooLong,
    #[error("a package has more than {MAX_VERSION_PREFIXES} version prefixes")]
    VersionPrefixes,
}

/// What a value is to the recipe reader, by the key it is under.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    Document,
    Packages,
    Package,
    VersionPrefix,
    Other,
}

impl Node {
    /// The value under `key`. `packages` holds the recipes only at the top of
    /// the document, which the walk checks.
    fn under(key: &str) -> Self {
        match key {
            "packages" => Self::Packages,
            "version_prefix" => Self::VersionPrefix,
            _ => Self::Other,
        }
    }
}

#[derive(Default)]
struct Walked {
    /// The distinct version prefixes of the package being walked.
    prefixes: Option<BTreeSet<String>>,
    /// The limit that stopped the walk. The decoder's error carries only text.
    broken: Option<Limit>,
}

impl Walked {
    fn text(&mut self, node: Node, text: &str) -> Result<(), Limit> {
        if text.contains("{{") {
            if !text.is_ascii() {
                return Err(Limit::TemplateNotAscii);
            }
            if text.len() > MAX_TEMPLATE_BYTES {
                return Err(Limit::TemplateTooLong);
            }
        }
        match (node, &mut self.prefixes) {
            (Node::VersionPrefix, Some(prefixes)) => {
                prefixes.insert(text.to_owned());
                match prefixes.len() > MAX_VERSION_PREFIXES {
                    true => Err(Limit::VersionPrefixes),
                    false => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }
}

struct Walk<'a> {
    walked: &'a mut Walked,
    node: Node,
}

impl Walk<'_> {
    fn at(&mut self, node: Node) -> Walk<'_> {
        Walk {
            walked: self.walked,
            node,
        }
    }
}

impl<'de> DeserializeSeed<'de> for Walk<'_> {
    /// What the value under this scalar is when the scalar is a key.
    type Value = Node;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Walk<'_> {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a YAML value")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Node, E> {
        match self.walked.text(self.node, text) {
            Ok(()) => Ok(Node::under(text)),
            Err(limit) => {
                self.walked.broken = Some(limit);
                Err(E::custom(limit))
            }
        }
    }

    fn visit_bool<E>(self, _: bool) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_i128<E>(self, _: i128) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_u128<E>(self, _: u128) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_bytes<E>(self, _: &[u8]) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_unit<E>(self) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_none<E>(self) -> Result<Node, E> {
        Ok(Node::Other)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<Node, A::Error> {
        let item = match self.node {
            Node::Packages => Node::Package,
            _ => Node::Other,
        };
        while seq.next_element_seed(self.at(item))?.is_some() {}
        Ok(Node::Other)
    }

    fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<Node, A::Error> {
        let package = self.node == Node::Package;
        if package {
            self.walked.prefixes = Some(BTreeSet::new());
        }
        while let Some(value) = map.next_key_seed(self.at(Node::Other))? {
            let value = match value {
                Node::Packages if self.node != Node::Document => Node::Other,
                value => value,
            };
            map.next_value_seed(self.at(value))?;
        }
        if package {
            self.walked.prefixes = None;
        }
        Ok(Node::Other)
    }
}

/// Checks `yaml` against the limits. A fault names its kind and where the
/// decoder was, never the text.
pub(super) fn check(yaml: &str) -> Result<(), Problem> {
    let mut walked = Walked::default();
    // The options that the recipe reader decodes a registry with, and no tag
    // that YAML does not define. The reader takes a node with such a tag as a
    // mapping from the tag to the node, which would give the node a key that
    // this walk does not see.
    let options = serde_saphyr::options! {
        strict_booleans: true,
        reject_unsupported_tags: true,
    };
    let walk = Walk {
        walked: &mut walked,
        node: Node::Document,
    };
    let result = serde_saphyr::with_deserializer_from_str_with_options(yaml, options, |document| {
        walk.deserialize(document)
    });
    match (result, walked.broken) {
        (Ok(_), _) => Ok(()),
        (Err(error), Some(limit)) => Err(Problem::Limit {
            limit,
            at: Position::of(&error),
        }),
        (Err(error), None) => Err(Problem::Malformed {
            at: Position::of(&error),
        }),
    }
}
