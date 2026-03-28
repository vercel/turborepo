fn main() -> Result<(), Box<dyn std::error::Error>> {
    let capnpc_result = capnpc::CompilerCommand::new()
        .file("./src/proto.capnp")
        .run();

    let invocation = std::env::var("RUSTC_WRAPPER").unwrap_or_default();
    if invocation.ends_with("rust-analyzer") {
        if capnpc_result.is_err() {
            println!("cargo:warning=capnpc failed, but continuing with rust-analyzer");
        }
        return Ok(());
    } else {
        capnpc_result.expect("schema compiler command");
    }

    Ok(())
}
