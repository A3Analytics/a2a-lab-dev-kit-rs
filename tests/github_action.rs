use std::path::Path;

const ACTION: &str = include_str!("../action.yml");
const EXAMPLE: &str = include_str!("../.github/examples/a2a-lab-compliance.yml");
const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const README: &str = include_str!("../README.md");
const MISE_TASKS: &str = include_str!("../.mise/run.toml");
const QUALITY_SCRIPT: &str = include_str!("../.mise/scripts/quality.sh");
const STANDALONE_TCK_SHA: &str = "1fd47d5c73e6fa3f7bd6505d75e43ee68cae33fd";
const LAST_EMBEDDED_SHA: &str = "9d5327868d96b3e800fd89f6debf434bcc12709d";

#[test]
fn legacy_action_forwards_its_complete_contract_to_the_standalone_tck() {
    assert!(ACTION.contains("::warning title=A2A-LAB Action moved::"));
    assert!(ACTION.contains(&format!(
        "uses: A3Analytics/a2a-lab-tck@{STANDALONE_TCK_SHA}"
    )));
    for input in [
        "a2a-url",
        "mcp-url",
        "fixtures",
        "profile",
        "suite",
        "llm-check",
        "report-path",
        "timeout-milliseconds",
        "implementation-name",
        "implementation-version",
    ] {
        assert!(
            ACTION.contains(&format!("{input}: ${{{{ inputs.{input} }}}}")),
            "legacy action does not forward {input}"
        );
    }
    for output in ["profile", "suite", "llm-check", "compliant", "report-path"] {
        assert!(
            ACTION.contains(&format!(
                "value: ${{{{ steps.compliance.outputs.{output} }}}}"
            )),
            "legacy action does not forward {output}"
        );
    }
    assert!(ACTION.contains("default: full"));
    assert!(ACTION.contains(r#"default: "true""#));
}

#[test]
fn public_guidance_uses_the_published_standalone_tck() {
    for document in [README, EXAMPLE] {
        assert!(document.contains("A3Analytics/a2a-lab-tck"));
        assert!(document.contains(STANDALONE_TCK_SHA));
        assert!(!document.contains("uses: A3Analytics/a2a-lab-dev-kit-rs@"));
    }
    assert!(README.contains(LAST_EMBEDDED_SHA));
    assert!(README.contains("deprecated compatibility bridge"));
    assert!(EXAMPLE.contains("actions/upload-artifact@v4"));
    assert!(EXAMPLE.contains("if: always()"));
    assert!(EXAMPLE.contains("not certification"));
}

#[test]
fn embedded_tck_implementation_cannot_reenter_the_devkit_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in [
        "src/compliance.rs",
        "src/compliance_runner.rs",
        "src/bin/a2a-lab-compliance.rs",
        ".github/actions/a2a-lab-compliance/run.sh",
        "tests/compliance_contract.rs",
        "tests/compliance_runner.rs",
        "schemas/compliance-fixtures-1.1.0.json",
        "schemas/compliance-result-1.1.0.json",
        "backlog/docs/guide/compliance/doc-22 - Adopt-A2A-LAB-compliance.md",
        "backlog/docs/reference/compliance/doc-23 - A2A-LAB-compliance-profile.md",
    ] {
        assert!(
            !root.join(path).exists(),
            "embedded TCK asset returned: {path}"
        );
    }
    assert!(!LIB.contains("mod compliance"));
    assert!(!MANIFEST.contains("a2a-lab-tck"));
    assert!(!MANIFEST.contains("a2a-lab-compliance"));
}

#[test]
fn official_a2a_tck_remains_in_the_quality_gate() {
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".github/workflows/a2a-tck.yml")
            .is_file()
    );
    assert!(MISE_TASKS.contains("[tck]"));
    assert!(MISE_TASKS.contains(".mise/scripts/a2a-tck.sh"));
    assert!(QUALITY_SCRIPT.contains("step tck bash .mise/scripts/a2a-tck.sh"));
}
