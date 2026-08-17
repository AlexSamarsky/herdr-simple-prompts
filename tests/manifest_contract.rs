#[test]
fn manifest_is_source_only_and_registers_targeted_zoomed_view() {
    let manifest = std::fs::read_to_string("herdr-plugin.toml").unwrap();
    let parsed: toml::Value = toml::from_str(&manifest).unwrap();

    assert_eq!(parsed["id"].as_str(), Some("herdr.simple-prompts"));
    assert_eq!(parsed["min_herdr_version"].as_str(), Some("0.7.5"));
    assert_eq!(
        strings(&parsed["build"][0]["command"]),
        vec!["cargo", "build", "--locked", "--release"]
    );
    assert_eq!(parsed["actions"][0]["id"].as_str(), Some("toggle"));
    assert_eq!(
        strings(&parsed["actions"][0]["command"]),
        vec!["./target/release/herdr-simple-prompts", "toggle"]
    );
    assert_eq!(parsed["panes"][0]["id"].as_str(), Some("simple-prompts"));
    assert_eq!(parsed["panes"][0]["placement"].as_str(), Some("zoomed"));
    assert_eq!(
        strings(&parsed["panes"][0]["command"]),
        vec!["./target/release/herdr-simple-prompts", "ui"]
    );
    assert!(!manifest.contains("curl"));
    assert!(!manifest.contains("wget"));
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect()
}

/// The declared version is the one the code actually needs, not the one the
/// edition alone would allow. `let` chains are stable from 1.88 and this
/// codebase leans on them throughout — in the style runs, in the ANSI slicing,
/// in the drawing — so a build pinned lower does not compile at all, which is
/// what the pinned CI job kept saying.
#[test]
fn dependencies_honor_the_declared_rust_version() {
    let cargo = std::fs::read_to_string("Cargo.toml").unwrap();
    let parsed: toml::Value = toml::from_str(&cargo).unwrap();

    assert_eq!(parsed["package"]["rust-version"].as_str(), Some("1.88"));
    assert_eq!(
        parsed["dependencies"]["ratatui"]["version"].as_str(),
        Some("0.29.0")
    );
    assert_eq!(parsed["dependencies"]["crossterm"].as_str(), Some("0.28.1"));
}
