//! Pinned Chart.js release fetched into the ignored `web-ui/vendor` cache.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const VERSION: &str = "4.5.1";
const ARCHIVE_URL: &str =
    "https://github.com/chartjs/Chart.js/releases/download/v4.5.1/chart.js-4.5.1.tgz";
const ARCHIVE_SHA256: &str = "d74ffdc9e4760d8960de3fd92bfa03397f01f85497c887d580bcf72641766aad";
const CHART_SHA256: &str = "48444a82d4edcb5bec0f1965faacdde18d9c17db3063d042abada2f705c9f54a";
const LICENSE_SHA256: &str = "41a84aa2caba645f966a18d9c2056b73e6d3a81d80bc0046bc0011a2634d4cce";
const CHART_MEMBER: &str = "package/dist/chart.umd.min.js";
const LICENSE_MEMBER: &str = "package/LICENSE.md";
const CHART_FILE: &str = "chart.umd.min.js";
const LICENSE_FILE: &str = "chart.js.LICENSE.md";

struct ChartFiles {
    chart: Vec<u8>,
    license: Vec<u8>,
}

pub fn prepare(vendor: &Path, allow_download: bool) -> Result<(), String> {
    if cache_is_valid(vendor) {
        return Ok(());
    }
    if !allow_download {
        return Err(format!(
            "web UI chart cache at {} is missing or invalid and downloads are disabled",
            vendor.display()
        ));
    }
    let archive = download(ARCHIVE_URL)?;
    require_hash(
        &archive,
        ARCHIVE_SHA256,
        &format!("Chart.js {VERSION} release archive"),
    )?;
    let extracted = extract_chart(&archive)?;
    require_hash(&extracted.chart, CHART_SHA256, "chart.umd.min.js")?;
    require_hash(&extracted.license, LICENSE_SHA256, "Chart.js license")?;
    write_file(&vendor.join(CHART_FILE), &extracted.chart)?;
    write_file(&vendor.join(LICENSE_FILE), &extracted.license)?;
    Ok(())
}

fn cache_is_valid(vendor: &Path) -> bool {
    cache_matches(vendor, CHART_SHA256, LICENSE_SHA256)
}

fn cache_matches(vendor: &Path, chart_hash: &str, license_hash: &str) -> bool {
    file_matches(&vendor.join(CHART_FILE), chart_hash)
        && file_matches(&vendor.join(LICENSE_FILE), license_hash)
}

fn file_matches(path: &Path, expected: &str) -> bool {
    fs::read(path).is_ok_and(|bytes| sha256(&bytes) == expected)
}

fn require_hash(bytes: &[u8], expected: &str, what: &str) -> Result<(), String> {
    let actual = sha256(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{what} checksum mismatch: expected {expected}, got {actual}"
        ))
    }
}

fn extract_chart(archive: &[u8]) -> Result<ChartFiles, String> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);
    let entries = tar
        .entries()
        .map_err(|error| format!("read Chart.js archive: {error}"))?;
    let mut chart = None;
    let mut license = None;
    for entry in entries {
        let mut entry = entry.map_err(|error| format!("read Chart.js archive entry: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("read Chart.js archive path: {error}"))?;
        let name = member_name(path.as_ref())?;
        if name != CHART_MEMBER && name != LICENSE_MEMBER {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(format!(
                "Chart.js archive member {name} is not a regular file"
            ));
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read Chart.js archive member {name}: {error}"))?;
        match name.as_str() {
            CHART_MEMBER => chart = Some(bytes),
            LICENSE_MEMBER => license = Some(bytes),
            _ => {}
        }
    }
    Ok(ChartFiles {
        chart: chart.ok_or_else(|| format!("Chart.js archive is missing {CHART_MEMBER}"))?,
        license: license.ok_or_else(|| format!("Chart.js archive is missing {LICENSE_MEMBER}"))?,
    })
}

fn member_name(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_start_matches("./");
    let safe = !text.is_empty()
        && !text.starts_with('/')
        && text
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if safe {
        Ok(text.to_owned())
    } else {
        Err(format!("Chart.js archive contains an unsafe path: {text}"))
    }
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("download Chart.js {VERSION} from {url}: {error}"))?;
    let status = response.status();
    if status != 200 {
        return Err(format!(
            "download Chart.js {VERSION} from {url} returned HTTP {status}"
        ));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read Chart.js {VERSION} archive: {error}"))?;
    Ok(bytes)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let temporary = temporary_path(path);
    fs::write(&temporary, bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        format!(
            "rename {} to {}: {error}",
            temporary.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    PathBuf::from(temporary)
}

fn sha256(bytes: &[u8]) -> String {
    super::freshness::sha256_hex(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{
        CHART_FILE, CHART_MEMBER, CHART_SHA256, LICENSE_FILE, LICENSE_MEMBER, cache_matches,
        extract_chart, prepare, require_hash, sha256,
    };

    #[test]
    fn checksum_rejects_a_changed_payload() {
        let error = require_hash(b"changed", CHART_SHA256, "chart.umd.min.js").unwrap_err();
        assert!(error.contains("checksum mismatch"));
        assert!(error.contains(CHART_SHA256));
    }

    #[test]
    fn checksum_accepts_the_expected_digest() {
        let bytes = b"chart.js";
        assert!(require_hash(bytes, &sha256(bytes), "chart.umd.min.js").is_ok());
    }

    #[test]
    fn extract_chart_reads_only_the_pinned_members() {
        let archive = archive(&[
            (CHART_MEMBER, b"chart-bytes"),
            (LICENSE_MEMBER, b"license-bytes"),
            ("package/README.md", b"ignore-me"),
        ]);
        let extracted = extract_chart(&archive).unwrap();
        assert_eq!(extracted.chart, b"chart-bytes");
        assert_eq!(extracted.license, b"license-bytes");
    }

    #[test]
    fn extract_chart_rejects_an_unsafe_path() {
        let archive = gzip(&raw_member("../outside.js", b"nope"));
        let Err(error) = extract_chart(&archive) else {
            panic!("unsafe archive was accepted");
        };
        assert!(error.contains("unsafe path"));
    }

    #[test]
    fn extract_chart_rejects_a_linked_bundle() {
        let archive = linked_archive();
        let Err(error) = extract_chart(&archive) else {
            panic!("linked bundle was accepted");
        };
        assert!(error.contains("not a regular file"));
    }

    #[test]
    fn cache_matches_requires_both_pinned_hashes() {
        let vendor = std::env::temp_dir().join(format!("web-ui-chart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vendor);
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join(CHART_FILE), b"chart").unwrap();
        std::fs::write(vendor.join(LICENSE_FILE), b"license").unwrap();
        assert!(cache_matches(
            &vendor,
            &sha256(b"chart"),
            &sha256(b"license")
        ));
        assert!(!cache_matches(
            &vendor,
            &sha256(b"other"),
            &sha256(b"license")
        ));
        assert!(!cache_matches(&vendor, &sha256(b"chart"), &sha256(b"nope")));
        let _ = std::fs::remove_dir_all(&vendor);
    }

    #[test]
    fn valid_cache_is_reused_without_downloading() {
        let vendor = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web-ui/vendor");
        prepare(&vendor, false).expect("valid chart cache");
    }

    #[test]
    fn disabled_download_does_not_replace_a_missing_cache() {
        let vendor = std::env::temp_dir().join(format!("web-ui-chart-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vendor);
        let error = prepare(&vendor, false).unwrap_err();
        assert!(error.contains("downloads are disabled"));
        assert!(!vendor.join(CHART_FILE).exists());
        let _ = std::fs::remove_dir_all(&vendor);
    }

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, *path, *bytes).unwrap();
        }
        gzip(&builder.into_inner().unwrap())
    }

    fn linked_archive() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_link(&mut header, CHART_MEMBER, "elsewhere.js")
            .unwrap();
        let mut license = tar::Header::new_gnu();
        license.set_size(0);
        license.set_mode(0o644);
        license.set_cksum();
        builder
            .append_data(&mut license, LICENSE_MEMBER, &b""[..])
            .unwrap();
        gzip(&builder.into_inner().unwrap())
    }

    fn raw_member(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{size:011o}\0", size = bytes.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = b'0';
        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        let checksum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(checksum.as_bytes());
        let mut block = header.to_vec();
        block.extend_from_slice(bytes);
        let padding = (512 - (bytes.len() % 512)) % 512;
        block.extend(std::iter::repeat_n(0, padding));
        block.extend_from_slice(&[0; 1024]);
        block
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
}
