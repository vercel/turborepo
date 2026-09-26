const STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() {
    // The CLI has historically needed over 1MB of stack space on MSVC.
    // Keep the larger stack until Windows-native testing confirms that the
    // current parser fits within the default 1MB stack.
    // https://learn.microsoft.com/en-us/windows/win32/procthread/thread-stack-size
    if std::env::var("CARGO_CFG_TARGET_ENV").ok().as_deref() == Some("msvc") {
        println!("cargo:rustc-link-arg=/stack:{STACK_SIZE}");
    }
}
