use std::io::BufRead as _;
use std::io::Write as _;

#[path = "../tests/helpers/mod.rs"]
mod helpers;

fn main() {
    let name = std::env::args().nth(1).unwrap();
    let _ = std::fs::remove_dir_all(format!("tests/data/fixtures/{name}"));
    std::fs::create_dir_all(format!("tests/data/fixtures/{name}")).unwrap();

    let inputs =
        std::fs::File::open(format!("tests/data/fixtures/{name}.in"))
            .unwrap();
    let inputs = std::io::BufReader::new(inputs);

    let mut i = 1;
    let mut prev_input = vec![];
    for line in inputs.lines() {
        let line = line.unwrap();

        let input = helpers::unhex(line.as_bytes());
        let mut input_file = std::fs::File::create(format!(
            "tests/data/fixtures/{name}/{i}.typescript"
        ))
        .unwrap();
        input_file.write_all(&input).unwrap();

        prev_input.extend(input);
        let mut term = vt100::Parser::default();
        term.process(&prev_input);
        let screen = helpers::FixtureScreen::from_screen(term.screen());

        let output_file = std::fs::File::create(format!(
            "tests/data/fixtures/{name}/{i}.json"
        ))
        .unwrap();
        serde_json::to_writer_pretty(output_file, &screen).unwrap();

        i += 1;
    }
}
