fn main() {
    println!("cargo:rerun-if-changed=proto");
    unsafe {
        std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    }
    let protos = [
        "proto/SiLAService.proto",
        "proto/SiLABinaryTransfer.proto",
        "proto/AnyTypeTest.proto",
        "proto/AuthenticationService.proto",
        "proto/AuthenticationTest.proto",
        "proto/AuthorizationService.proto",
        "proto/BasicDataTypesTest.proto",
        "proto/BinaryTransferTest.proto",
        "proto/ErrorHandlingTest.proto",
        "proto/ListDataTypeTest.proto",
        "proto/MetadataConsumerTest.proto",
        "proto/MetadataProvider.proto",
        "proto/MultiClientTest.proto",
        "proto/ObservableCommandTest.proto",
        "proto/ObservablePropertyTest.proto",
        "proto/StructureDataTypeTest.proto",
        "proto/UnobservableCommandTest.proto",
        "proto/UnobservablePropertyTest.proto",
    ];
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(&protos, &["proto"])
        .expect("generate communication-tester stubs");
}
