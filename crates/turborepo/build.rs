const STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() {
    // clap's proc macro uses stack allocated arrays for parsing cli args
    // We have a large enough CLI that we attempt to put over 1MB onto the stack.
    // This causes an issue on Windows where the default stack size is 1MB
    // See
    // https://learn.microsoft.com/en-us/windows/win32/procthread/thread-stack-size
    // https://github.com/clap-rs/clap/issues/5134
    if std::env::var("CARGO_CFG_TARGET_ENV").ok().as_deref() == Some("msvc") {
        println!("cargo:rustc-link-arg=/stack:{STACK_SIZE}");
    }
}
