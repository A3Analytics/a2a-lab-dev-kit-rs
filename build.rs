fn main() {
    println!("cargo:rerun-if-changed=proto/sila/lab.proto");
    #[cfg(feature = "sila2")]
    unsafe {
        std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    }
    #[cfg(feature = "sila2")]
    tonic_prost_build::configure()
        .build_server(true)
        .compile_protos(&["proto/sila/lab.proto"], &["proto/sila"])
        .expect("generate SiLA lab client");
}
