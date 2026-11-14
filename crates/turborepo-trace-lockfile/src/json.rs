//! serde helpers for the JSON-based formats.

use std::{borrow::Cow, fmt, marker::PhantomData};

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// A JSON object deserialized into a `Vec` of entries in document order (no
/// hashing, keeps borrowed keys).
#[derive(Debug)]
pub(crate) struct MapVec<K, V>(pub(crate) Vec<(K, V)>);

impl<K, V> Default for MapVec<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de> for MapVec<K, V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V2<K, V>(PhantomData<(K, V)>);
        impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> Visitor<'de> for V2<K, V> {
            type Value = MapVec<K, V>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(e) = map.next_entry()? {
                    out.push(e);
                }
                Ok(MapVec(out))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(MapVec(Vec::new()))
            }
        }
        d.deserialize_any(V2(PhantomData))
    }
}

/// A string or an array of strings (npm `os`/`cpu`/`libc`, bun `os`/`cpu`).
#[derive(Debug, Default)]
pub(crate) struct OneOrMany<'a>(pub(crate) Vec<Cow<'a, str>>);

impl OneOrMany<'_> {
    pub(crate) fn to_vec(&self) -> Vec<String> {
        self.0
            .iter()
            .filter(|s| !s.is_empty() && *s != "none")
            .map(|s| s.to_string())
            .collect()
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for OneOrMany<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V1<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V1<'a> {
            type Value = OneOrMany<'a>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string or an array of strings")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(OneOrMany(vec![Cow::Borrowed(v)]))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(OneOrMany(vec![Cow::Owned(v.to_string())]))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(OneOrMany(Vec::new()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(s) = seq.next_element::<Str<'a>>()? {
                    out.push(s.0);
                }
                Ok(OneOrMany(out))
            }
        }
        d.deserialize_any(V1(PhantomData))
    }
}

/// A string that borrows from the JSON input when it has no escapes. (serde's
/// own `Cow<str>` impl always allocates unless it is a direct
/// `#[serde(borrow)]` field.)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Str<'a>(pub(crate) Cow<'a, str>);

impl std::ops::Deref for Str<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for Str<'_> {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Str<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V1<'a>(PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V1<'a> {
            type Value = Str<'a>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Str(Cow::Borrowed(v)))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Str(Cow::Owned(v.to_string())))
            }
        }
        d.deserialize_str(V1(PhantomData))
    }
}

/// Strips `//` and `/* */` comments and trailing commas from JSONC (bun.lock),
/// keeping string contents intact.
pub(crate) fn strip_jsonc(src: &str) -> Cow<'_, str> {
    let b = src.as_bytes();
    if !b.iter().any(|&c| c == b',' || c == b'/') {
        return Cow::Borrowed(src);
    }
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.extend_from_slice(&b[start..i]);
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            b',' => {
                let mut j = i + 1;
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if b.get(j) == Some(&b'/') && b.get(j + 1) == Some(&b'/') {
                        while j < b.len() && b[j] != b'\n' {
                            j += 1;
                        }
                    } else if b.get(j) == Some(&b'/') && b.get(j + 1) == Some(&b'*') {
                        j += 2;
                        while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') {
                            j += 1;
                        }
                        j = (j + 2).min(b.len());
                    } else {
                        break;
                    }
                }
                if !matches!(b.get(j), Some(b'}' | b']')) {
                    out.push(b',');
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    // Only ASCII bytes were removed outside strings, so the result stays valid
    // UTF-8.
    Cow::Owned(String::from_utf8(out).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc() {
        let s = strip_jsonc("{\"a\": [1, 2,], // c\n \"b,\": \"x//y\", /* z */ }");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["a"][1], 2);
        assert_eq!(v["b,"], "x//y");
    }
}
