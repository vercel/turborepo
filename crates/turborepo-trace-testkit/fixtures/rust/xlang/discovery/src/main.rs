//! Plugin discovery by listing a directory anchored at the crate root.
fn main() {
    let dir = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/plugins")).unwrap();
    let mut names: Vec<_> = dir.map(|e| e.unwrap().path()).collect();
    names.sort();
    for p in names {
        println!("{}", std::fs::read_to_string(p).unwrap().len());
    }
}
