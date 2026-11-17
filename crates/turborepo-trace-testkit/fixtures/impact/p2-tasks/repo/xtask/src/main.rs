//! Developer tasks: `cargo xtask codegen`.
mod codegen;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("codegen") => codegen::run(),
        _ => eprintln!("usage: cargo xtask codegen"),
    }
}
