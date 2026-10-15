use std::env;
use std::fs;
use std::path::PathBuf;

mod assemble;
mod freshness;
mod vendor;

pub fn package() {
    println!("cargo:rerun-if-env-changed=WEB_UI_CHECK_ONLY");
    println!("cargo:rerun-if-env-changed=WEB_UI_FRESHNESS_NONCE");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let ui_dir = manifest_dir.join("web-ui");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    for path in freshness::source_files(&ui_dir) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let fingerprint = freshness::fingerprint_tree(&ui_dir);
    let stamp_path = manifest_dir.join("target/web-ui-wasm/source.fingerprint");
    let pkg = manifest_dir.join("target/web-ui-wasm/pkg");
    let recorded = fs::read_to_string(&stamp_path).ok();
    let status = freshness::package_status(
        &fingerprint,
        recorded.as_deref(),
        assemble::artifacts_present(&pkg),
    );
    if status != freshness::PackageStatus::Fresh {
        assert!(
            env::var_os("WEB_UI_CHECK_ONLY").is_none(),
            "stale generated web UI assets: packaged {recorded:?}, sources {fingerprint}"
        );
        vendor::prepare(&ui_dir.join("vendor"), true).unwrap_or_else(|error| {
            panic!("prepare web UI chart: {error}");
        });
        let staging = assemble::compile(&ui_dir, &out_dir);
        let files = assemble::assemble(&ui_dir, &staging, &pkg);
        assert!(
            !files.is_empty() && assemble::artifacts_present(&pkg),
            "web UI packaging did not produce a complete asset set"
        );
        if let Some(parent) = stamp_path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|error| panic!("create {}: {error}", parent.display()));
        }
        fs::write(&stamp_path, &fingerprint)
            .unwrap_or_else(|error| panic!("write {}: {error}", stamp_path.display()));
    }
    assemble::write_assets_rs(&out_dir, &pkg, &fingerprint);
}
