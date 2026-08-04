fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &["proto/aletheon/policy/gateway/v1/policy.proto"],
            &["proto"],
        )?;
    println!("cargo:rerun-if-changed=proto/aletheon/policy/gateway/v1/policy.proto");
    Ok(())
}
