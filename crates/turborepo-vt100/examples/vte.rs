use std::io::Read as _;

/// A type implementing Perform that just logs actions
struct Log;

impl vte::Perform for Log {
    fn print(&mut self, c: char) {
        println!("[print] U+{:04x}", c as u32);
    }

    fn execute(&mut self, byte: u8) {
        println!("[execute] {byte:02x}");
    }

    fn hook(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        c: char,
    ) {
        println!(
            "[hook] params={:?}, intermediates={:?}, ignore={:?}, c=U+{:04x}",
            params, intermediates, ignore, c as u32
        );
    }

    fn put(&mut self, byte: u8) {
        println!("[put] {byte:02x}");
    }

    fn unhook(&mut self) {
        println!("[unhook]");
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        println!(
            "[osc_dispatch] params={params:?} bell_terminated={bell_terminated}"
        );
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        c: char,
    ) {
        println!(
            "[csi_dispatch] \
            params={:#?}, intermediates={:?}, ignore={:?}, c=U+{:04x}",
            params, intermediates, ignore, c as u32
        );
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        println!(
            "[esc_dispatch] intermediates={intermediates:?}, ignore={ignore:?}, byte={byte:02x}"
        );
    }
}

fn main() {
    let mut stdin = std::io::stdin();
    let mut parser = vte::Parser::new();
    let mut performer = Log;

    let mut buf = [0; 4096];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for byte in &buf[..n] {
                    parser.advance(&mut performer, *byte);
                }
            }
            Err(err) => {
                eprintln!("err: {err}");
                std::process::exit(1);
            }
        }
    }
}
