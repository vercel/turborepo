//! Writes src/generated/version.js.
pub fn run() {
    let v = "1.0.0";
    std::fs::write("src/generated/version.js", format!("module.exports = {{ version: {v:?} }};\n"))
        .expect("write version.js");
}
