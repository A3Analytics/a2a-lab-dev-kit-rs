use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use a2a_lab_dev_kit::{
    COMPLIANCE_PROFILE_VERSION, ComplianceFixtures, ComplianceRunnerConfig, ComplianceSuite,
    ImplementationIdentity, run_compliance,
};

#[tokio::main]
async fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let fixtures_json = match std::fs::read_to_string(&options.fixtures) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("could not read fixtures: {error}");
            return ExitCode::from(2);
        }
    };
    let fixtures = match ComplianceFixtures::from_json(&fixtures_json) {
        Ok(fixtures) => fixtures,
        Err(error) => {
            eprintln!("invalid fixtures: {error}");
            return ExitCode::from(2);
        }
    };
    let config = ComplianceRunnerConfig {
        a2a_url: options.a2a_url,
        mcp_url: options.mcp_url,
        fixtures,
        implementation: ImplementationIdentity {
            name: options.implementation_name,
            version: options.implementation_version,
        },
        case_timeout: Duration::from_millis(options.timeout_milliseconds),
        suite: options.suite,
        agent_message_check: options.llm_check,
    };
    let report = match run_compliance(config).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let json = match serde_json::to_string_pretty(&report) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("could not encode report: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = std::fs::write(&options.report, format!("{json}\n")) {
        eprintln!("could not write report: {error}");
        return ExitCode::from(2);
    }
    if report.compliant {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

struct Options {
    a2a_url: String,
    mcp_url: String,
    fixtures: PathBuf,
    report: PathBuf,
    implementation_name: String,
    implementation_version: String,
    timeout_milliseconds: u64,
    suite: ComplianceSuite,
    llm_check: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let mut a2a_url = None;
        let mut mcp_url = None;
        let mut fixtures = None;
        let mut report = None;
        let mut implementation_name = None;
        let mut implementation_version = None;
        let mut timeout_milliseconds = 30_000;
        let mut suite = ComplianceSuite::Full;
        let mut llm_check = true;
        while let Some(flag) = args.next() {
            if flag == "--no-llm-check" {
                llm_check = false;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}\n{}", usage()))?;
            match flag.as_str() {
                "--a2a-url" => a2a_url = Some(value),
                "--mcp-url" => mcp_url = Some(value),
                "--fixtures" => fixtures = Some(PathBuf::from(value)),
                "--report" => report = Some(PathBuf::from(value)),
                "--implementation-name" => implementation_name = Some(value),
                "--implementation-version" => implementation_version = Some(value),
                "--suite" => {
                    suite = match value.as_str() {
                        "basic" => ComplianceSuite::Basic,
                        "full" => ComplianceSuite::Full,
                        _ => {
                            return Err(format!(
                                "unsupported suite `{value}`; expected basic or full"
                            ));
                        }
                    };
                }
                "--profile" if value == COMPLIANCE_PROFILE_VERSION => {}
                "--profile" => {
                    return Err(format!(
                        "unsupported profile `{value}`; expected {COMPLIANCE_PROFILE_VERSION}"
                    ));
                }
                "--timeout-milliseconds" => {
                    timeout_milliseconds = value
                        .parse()
                        .map_err(|_| "--timeout-milliseconds must be an integer".to_owned())?;
                }
                _ => return Err(format!("unknown argument `{flag}`\n{}", usage())),
            }
        }
        Ok(Self {
            a2a_url: required(a2a_url, "--a2a-url")?,
            mcp_url: required(mcp_url, "--mcp-url")?,
            fixtures: required(fixtures, "--fixtures")?,
            report: required(report, "--report")?,
            implementation_name: required(implementation_name, "--implementation-name")?,
            implementation_version: required(implementation_version, "--implementation-version")?,
            timeout_milliseconds,
            suite,
            llm_check,
        })
    }
}

fn required<T>(value: Option<T>, flag: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("missing required argument {flag}\n{}", usage()))
}

fn usage() -> &'static str {
    "usage: a2a-lab-compliance --a2a-url URL --mcp-url URL --fixtures FILE --report FILE \
     --implementation-name NAME --implementation-version VERSION \
     [--profile 1.1.0] [--suite basic|full] [--no-llm-check] [--timeout-milliseconds N]"
}
