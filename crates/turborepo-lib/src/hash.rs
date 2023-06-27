extern crate serde;

use rmp_serde::{encode::Error, Serializer};
use serde::Serialize;
use xxhash_rust::xxh64::xxh64;

fn hash_object(o: impl Serialize) -> Result<String, Error> {
    let mut buf = Vec::new();
    o.serialize(&mut Serializer::new(&mut buf).with_struct_map())?;
    let sum = xxh64(buf.as_slice(), 0);
    println!("{:?}", buf.as_slice());
    let hash = format!("{:x}", sum);
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(non_snake_case)] // copied from a Go struct
    #[derive(Debug, Serialize)]
    struct TaskOutputs {
        Inclusions: Vec<String>,
        Exclusions: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    struct Complex {
        Nested: TaskOutputs,
        Foo: String,
        Bar: Vec<String>,
    }

    #[test]
    fn test_hash_object() {
        // let task_object = TaskOutputs {
        //     Inclusions: vec!["foo".to_string(), "bar".to_string()],
        //     Exclusions: vec!["baz".to_string()]
        // };

        // let hash = hash_object(&task_object).unwrap();
        // assert_eq!(hash, "6ea4cef295ea772c");

        let complex = Complex {
            Nested: TaskOutputs {
                Exclusions: vec!["bar".to_string(), "baz".to_string()],
                Inclusions: vec!["foo".to_string()],
            },
            Foo: "a".to_string(),
            Bar: vec!["b".to_string(), "c".to_string()],
        };
        let hash = hash_object(&complex).unwrap();
        assert_eq!(hash, "d55de0d0e0944858");
    }
}
