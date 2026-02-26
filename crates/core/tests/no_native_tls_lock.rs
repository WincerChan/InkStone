use std::path::PathBuf;

fn workspace_lockfile() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")
}

#[test]
fn workspace_lockfile_does_not_include_native_tls_or_openssl_chain() {
    let lockfile = std::fs::read_to_string(workspace_lockfile())
        .expect("failed to read workspace Cargo.lock");

    for forbidden in [
        "name = \"native-tls\"",
        "name = \"hyper-tls\"",
        "name = \"tokio-native-tls\"",
        "name = \"openssl\"",
    ] {
        assert!(
            !lockfile.contains(forbidden),
            "Cargo.lock still contains forbidden package entry: {forbidden}",
        );
    }
}
