use std::io::BufRead as _;
use std::io::Write as _;

#[path = "../tests/helpers/mod.rs"]
mod helpers;

fn main() {
    let name = std::env::args().nth(1).unwrap();
    let _ = std::fs::remove_file(format!("fuzz/in/{name}"));

    let inputs =
        std::fs::File::open(format!("tests/data/fixtures/{name}.in"))
            .unwrap();
    let inputs = std::io::BufReader::new(inputs);

    let mut bytes = vec![];
    for line in inputs.lines() {
        let line = line.unwrap();
        let input = helpers::unhex(line.as_bytes());
        bytes.extend(input.iter());
    }
    let mut input_file =
        std::fs::File::create(format!("fuzz/in/{name}")).unwrap();
    input_file.write_all(&bytes).unwrap();
}
