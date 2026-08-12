#[test]
fn manifest_is_source_only_and_registers_toggle_overlay() {
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
    assert_eq!(parsed["panes"][0]["placement"].as_str(), Some("overlay"));
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

#[test]
fn dependencies_honor_the_declared_rust_version() {
    let cargo = std::fs::read_to_string("Cargo.toml").unwrap();
    let parsed: toml::Value = toml::from_str(&cargo).unwrap();

    assert_eq!(parsed["package"]["rust-version"].as_str(), Some("1.85"));
    assert_eq!(
        parsed["dependencies"]["ratatui"]["version"].as_str(),
        Some("0.29.0")
    );
    assert_eq!(parsed["dependencies"]["crossterm"].as_str(), Some("0.28.1"));
}
