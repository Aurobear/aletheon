fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        // The generated server is used by in-process protocol acceptance
        // fixtures. Production composition still exports only the client.
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &["proto/aletheon/policy/gateway/v1/policy.proto"],
            &["proto"],
        )?;
    println!("cargo:rerun-if-changed=proto/aletheon/policy/gateway/v1/policy.proto");
    Ok(())
}
