//! A dynamic library loaded with libloading (static fixture: the crate is not vendored).
fn main() {
    let lib = unsafe { libloading::Library::new("plugins/libgreeter.so") };
    let sys = unsafe { libloading::Library::new("libz.so.1") };
    println!("{} {}", lib.is_ok(), sys.is_ok());
}
