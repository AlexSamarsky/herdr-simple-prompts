#[test]
fn manifest_is_source_only_and_registers_toggle_overlay() {
    let manifest = std::fs::read_to_string("herdr-plugin.toml").unwrap();

    assert!(manifest.contains("id = \"herdr.simple-prompts\""));
    assert!(manifest.contains("min_herdr_version = \"0.7.5\""));
    assert!(manifest.contains("command = [\"cargo\", \"build\", \"--locked\", \"--release\"]"));
    assert!(manifest.contains("id = \"toggle\""));
    assert!(manifest.contains("id = \"simple-prompts\""));
    assert!(manifest.contains("placement = \"overlay\""));
    assert!(!manifest.contains("curl"));
    assert!(!manifest.contains("wget"));
}
