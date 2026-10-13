use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::freshness::short_hash;

pub fn compile(ui_dir: &Path, out_dir: &Path) -> PathBuf {
    let target_dir = ui_dir.parent().unwrap_or(ui_dir).join("target/web-ui-wasm");
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
    command
        .arg("build")
        .arg("--locked")
        .arg("--release")
        .arg("--manifest-path")
        .arg(ui_dir.join("Cargo.toml"))
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTFLAGS");
    if std::env::var("RUSTC").is_ok_and(|rustc| rustc.contains("clippy")) {
        command.env_remove("RUSTC");
    }
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("spawn wasm32 browser build: {error}"));
    assert!(
        status.success(),
        "wasm32 browser build failed with {status}"
    );
    let wasm = target_dir.join("wasm32-unknown-unknown/release/a2a_lab_web_ui.wasm");
    assert!(wasm.is_file(), "missing wasm artifact {}", wasm.display());
    let staging = out_dir.join("web-ui-bindgen");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .unwrap_or_else(|error| panic!("create {}: {error}", staging.display()));
    let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
    bindgen
        .input_path(&wasm)
        .out_name("app")
        .typescript(false)
        .debug(false)
        .omit_default_module_path(false)
        .web(true)
        .unwrap_or_else(|error| panic!("configure wasm-bindgen web target: {error}"))
        .generate(&staging)
        .unwrap_or_else(|error| panic!("generate wasm-bindgen package: {error}"));
    assert!(
        !staging.join("snippets").exists(),
        "web UI foundation does not package wasm-bindgen snippets"
    );
    staging
}

pub fn assemble(ui_dir: &Path, staging: &Path, pkg: &Path) -> Vec<String> {
    let _ = fs::remove_dir_all(pkg);
    let assets = pkg.join("assets");
    fs::create_dir_all(&assets)
        .unwrap_or_else(|error| panic!("create {}: {error}", assets.display()));
    let wasm_bytes = fs::read(staging.join("app_bg.wasm"))
        .unwrap_or_else(|error| panic!("read generated wasm: {error}"));
    let wasm_name = format!("app-{}.wasm", short_hash(&wasm_bytes));
    let js = fs::read_to_string(staging.join("app.js"))
        .unwrap_or_else(|error| panic!("read generated javascript: {error}"));
    assert!(
        js.contains("app_bg.wasm"),
        "generated javascript does not reference app_bg.wasm"
    );
    let js = js.replace("app_bg.wasm", &wasm_name);
    let js_name = format!("app-{}.js", short_hash(js.as_bytes()));
    let css = fs::read(ui_dir.join("static/app.css"))
        .unwrap_or_else(|error| panic!("read web UI stylesheet: {error}"));
    let css_name = format!("app-{}.css", short_hash(&css));
    let logo = fs::read(ui_dir.join("static/logo.svg"))
        .unwrap_or_else(|error| panic!("read web UI logo: {error}"));
    let logo_name = format!("logo-{}.svg", short_hash(&logo));
    let boot = format!("import init from \"./{js_name}\";\nawait init();\n");
    let boot_name = format!("boot-{}.js", short_hash(boot.as_bytes()));
    let template = fs::read_to_string(ui_dir.join("static/index.html"))
        .unwrap_or_else(|error| panic!("read web UI shell: {error}"));
    let html = template
        .replace("WEB_UI_CSS", &css_name)
        .replace("WEB_UI_BOOT", &boot_name)
        .replace("WEB_UI_LOGO", &logo_name);
    assert!(
        !html.contains("WEB_UI_"),
        "web UI shell still contains unsubstituted placeholders"
    );
    fs::write(assets.join(&wasm_name), wasm_bytes).unwrap_or_else(|error| panic!("{error}"));
    fs::write(assets.join(&js_name), js).unwrap_or_else(|error| panic!("{error}"));
    fs::write(assets.join(&css_name), css).unwrap_or_else(|error| panic!("{error}"));
    fs::write(assets.join(&logo_name), logo).unwrap_or_else(|error| panic!("{error}"));
    fs::write(assets.join(&boot_name), boot).unwrap_or_else(|error| panic!("{error}"));
    fs::write(pkg.join("index.html"), html).unwrap_or_else(|error| panic!("{error}"));
    let mut files = vec![
        "index.html".to_owned(),
        format!("assets/{boot_name}"),
        format!("assets/{js_name}"),
        format!("assets/{wasm_name}"),
        format!("assets/{css_name}"),
        format!("assets/{logo_name}"),
    ];
    files.sort();
    fs::write(pkg.join("manifest.txt"), files.join("\n"))
        .unwrap_or_else(|error| panic!("write web UI manifest: {error}"));
    files
}

pub fn artifacts_present(pkg: &Path) -> bool {
    let Ok(manifest) = fs::read_to_string(pkg.join("manifest.txt")) else {
        return false;
    };
    manifest
        .lines()
        .filter(|line| !line.is_empty())
        .all(|line| pkg.join(line).is_file())
}

pub fn write_assets_rs(out_dir: &Path, pkg: &Path, fingerprint: &str) {
    let mut assets = Vec::new();
    for entry in fs::read_dir(pkg.join("assets"))
        .unwrap_or_else(|error| panic!("read packaged assets: {error}"))
    {
        let path = entry
            .unwrap_or_else(|error| panic!("read packaged asset: {error}"))
            .path();
        if path.is_file() {
            assets.push(path);
        }
    }
    assets.sort();
    let mut code = String::new();
    code.push_str("pub struct PackedAsset {\n");
    code.push_str("    pub name: &'static str,\n");
    code.push_str("    pub content_type: &'static str,\n");
    code.push_str("    pub body: &'static [u8],\n");
    code.push_str("}\n\n");
    write!(
        code,
        "#[allow(dead_code)]\npub const SOURCE_FINGERPRINT: &str = \"{fingerprint}\";\n\n"
    )
    .expect("format web UI asset module");
    write!(
        code,
        "pub const ENTRY_HTML: &[u8] = include_bytes!(\"{}\");\n\n",
        rust_path(&pkg.join("index.html"))
    )
    .expect("format web UI asset module");
    code.push_str("pub static ASSETS: &[PackedAsset] = &[\n");
    for path in &assets {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("web UI asset name is not utf-8"));
        writeln!(
            code,
            "    PackedAsset {{ name: \"{name}\", content_type: \"{}\", body: include_bytes!(\"{}\") }},",
            content_type(name),
            rust_path(path)
        )
        .expect("format web UI asset module");
    }
    code.push_str("];\n");
    fs::write(out_dir.join("web_ui_assets.rs"), code)
        .unwrap_or_else(|error| panic!("write web_ui_assets.rs: {error}"));
}

fn content_type(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some(extension) if extension.eq_ignore_ascii_case("js") => "text/javascript; charset=utf-8",
        Some(extension) if extension.eq_ignore_ascii_case("wasm") => "application/wasm",
        Some(extension) if extension.eq_ignore_ascii_case("css") => "text/css; charset=utf-8",
        Some(extension) if extension.eq_ignore_ascii_case("svg") => "image/svg+xml",
        _ => panic!("unknown web UI asset type: {name}"),
    }
}

fn rust_path(path: &Path) -> String {
    path.to_str()
        .unwrap_or_else(|| panic!("web UI path is not utf-8: {}", path.display()))
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}
