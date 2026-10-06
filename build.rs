fn main() {
    println!("cargo:rerun-if-changed=proto/sila");
    println!("cargo:rerun-if-changed=src/sila/standard");
    #[cfg(feature = "sila2")]
    unsafe {
        std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    }
    #[cfg(feature = "sila2")]
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/sila/SiLAService.proto",
                "proto/sila/LabOperations.proto",
                "proto/sila/CancelController.proto",
                "proto/sila/ConnectionConfigurationService.proto",
                "proto/sila/SiLABinaryTransfer.proto",
                "proto/sila/SiLACloudConnector.proto",
            ],
            &["proto/sila"],
        )
        .expect("generate SiLA server stubs");
}
