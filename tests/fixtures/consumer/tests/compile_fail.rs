use std::{path::PathBuf, process::Command};

#[test]
fn reports_stable_terms_for_graph_and_declaration_errors() {
    let cases = [
        ("case-missing", "missing provider for"),
        ("case-ambiguous", "ambiguous provider for"),
        ("case-cycle", "provider dependency cycle"),
        ("case-invalid-qualifier", "invalid qualifier for"),
        (
            "case-unsupported-syntax",
            "generic provider functions are not supported",
        ),
        ("case-duplicate-provider", "duplicate provider for"),
        ("case-missing-register-impl", "Register<"),
        ("case-missing-register-impl-unreachable", "Register<"),
        (
            "case-registration-target-missing",
            "missing provider for type",
        ),
        (
            "case-registration-target-ambiguous",
            "ambiguous provider for type",
        ),
        (
            "case-registration-target-invalid-qualifier",
            "invalid qualifier for type",
        ),
        (
            "case-registration-target-unreachable",
            "is not reachable from any hook",
        ),
        (
            "case-registration-self-target",
            "a provider cannot register itself",
        ),
        (
            "case-duplicate-registration",
            "duplicate registration capability",
        ),
        (
            "case-registration-bad-syntax",
            "expected `qualifier = \"name\"` after registration target type",
        ),
    ];
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../compile-fail/Cargo.toml");
    let target_dir = std::env::temp_dir().join(format!(
        "framework-composition-ui-target-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&target_dir).expect("temporary compile-fail target dir should exist");

    for (feature, expected_message) in cases {
        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet", "--offline", "--manifest-path"])
            .arg(&manifest)
            .args(["--features", feature, "--target-dir"])
            .arg(&target_dir)
            .env("CARGO_TERM_COLOR", "never")
            .output()
            .expect("cargo check should launch for compile-fail fixture");

        let diagnostics = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "fixture `{feature}` unexpectedly compiled:\n{diagnostics}"
        );
        assert!(
            diagnostics.contains(expected_message),
            "fixture `{feature}` did not contain `{expected_message}`:\n{diagnostics}"
        );
    }
}
