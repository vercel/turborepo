fn main() -> Result<(), Box<dyn std::error::Error>> {
    let result = capnpc::CompilerCommand::new()
        .file("./src/proto.capnp")
        .run();

    if std::env::var("RUSTC_WRAPPER")
        .unwrap_or_default()
        .ends_with("rust-analyzer")
        && result.is_err()
    {
        println!("cargo:warning=capnpc failed, but continuing with rust-analyzer");
        return Ok(());
    }

    result?;
    Ok(())
}
