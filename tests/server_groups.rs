use pulseboard::config;

const GROUPS: &str = r#"
    [[checks]]
    id = "live"
    name = "Standalone"
    url = "https://other.example/health"

    [[servers]]
    id = "production"
    name = "Production"
    base_url = "https://example.com"

    [[servers.checks]]
    id = "live"
    name = "API live"
    path = "/api/live"
    method = "get"
    expected_results = [200, 204]

    [[servers.checks]]
    id = "ready"
    name = "API ready"
    url = "https://services.example/ready"
    method = "POST"
    body = '{"component":"database"}'
    [servers.checks.headers_from_env]
    Authorization = "PROBE_AUTH"

    [[servers]]
    id = "staging"
    name = "Staging"
    base_url = "https://staging.example"

    [[servers.checks]]
    id = "live"
    name = "API live"
    path = "/api/live"
"#;

#[test]
fn groups_scope_check_ids_and_support_legacy_entries() {
    let config = config::parse(GROUPS).unwrap();
    assert_eq!(config.checks.len(), 4);
    assert_eq!(config.checks[0].key.server, None);
    assert_eq!(config.checks[1].key.server.as_deref(), Some("production"));
    assert_eq!(config.checks[1].request.expected_status, [200, 204]);
    assert_eq!(
        config.checks[1].url.as_str(),
        "https://example.com/api/live"
    );
    assert_eq!(
        config.checks[2].url.as_str(),
        "https://services.example/ready"
    );
    assert_eq!(
        config.checks[3].url.as_str(),
        "https://staging.example/api/live"
    );
    assert_eq!(config.checks[1].key.id, config.checks[3].key.id);
    assert!(config.checks[1].key != config.checks[3].key);
}

#[test]
fn copied_toml_can_be_pasted_back_to_duplicate_servers_and_checks() {
    let config = config::parse(GROUPS).unwrap();
    let server_block = config.copy_server_toml("production").unwrap();
    let check_block = config.copy_check_toml(&config.checks[2].key).unwrap();
    let edited = format!("{GROUPS}\n{server_block}\n{check_block}");
    let copied = config::parse(&edited).unwrap();
    assert_eq!(copied.servers.len(), 3);
    assert_eq!(copied.servers[2].id, "production-copy");
    assert_eq!(copied.checks.len(), 7);
    assert_eq!(copied.checks[4].key.id, "live");
    assert!(copied.checks[5].request == copied.checks[2].request);
    assert_eq!(copied.checks[6].key.id, "ready-copy");
    assert_eq!(
        copied.checks[6].request.body.as_deref(),
        Some(r#"{"component":"database"}"#)
    );
    assert_eq!(
        copied.checks[6].request.headers_from_env["Authorization"],
        "PROBE_AUTH"
    );
    assert!(
        copied
            .copy_server_toml("production")
            .unwrap()
            .contains("production-copy-2")
    );
}

#[test]
fn rejects_a_path_that_switches_hosts_and_duplicate_ids_in_one_group() {
    let wrong_host = GROUPS.replace("/api/live", "//other.example/health");
    let Err(error) = config::parse(&wrong_host) else {
        panic!("cross-host path accepted")
    };
    assert!(format!("{error:#}").contains("path muss"));

    let duplicate_id = GROUPS.replace("id = \"ready\"", "id = \"live\"");
    let Err(error) = config::parse(&duplicate_id) else {
        panic!("duplicate check ID accepted")
    };
    assert!(format!("{error:#}").contains("eindeutige"));
}
