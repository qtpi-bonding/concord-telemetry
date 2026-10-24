use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor = out.join("descriptor.bin");
    let includes = [
        PathBuf::from("proto"),
        concord_telemetry_proto::proto_include_dir(),
    ];

    prost_build::Config::new()
        .file_descriptor_set_path(&descriptor)
        .extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")
        .compile_protos(&["proto/consumer/v1/events.proto"], &includes)?;

    pbjson_build::Builder::new()
        .register_descriptors(&std::fs::read(&descriptor)?)?
        .extern_path(".concord_telemetry.v1", "::concord_telemetry::proto::v1")
        .build(&[".consumer.v1"])?;
    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
