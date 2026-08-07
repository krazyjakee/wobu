use super::*;
use std::sync::atomic::AtomicUsize;
use wobu_mcp::config::DEFAULT_PORT;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("wobu-mcp-{}-{name}-{n}.json", std::process::id()))
}

#[test]
fn a_fresh_installation_has_nothing_enabled_and_no_token_on_disk() {
    // The claim the privacy policy makes, at the only layer that can make
    // it: a Wobu nobody has configured has no listener and no credential.
    let path = scratch("fresh");
    let state = McpState::load_from(path.clone());
    let view = state.view();
    assert!(!view.server.enabled);
    assert!(!view.server.running);
    assert!(!view.server.allow_writes);
    assert!(view.server.token_preview.is_none());
    assert!(!view.client.enabled);
    assert!(view.client.servers.is_empty());
    assert!(!path.exists(), "loading settings must not write a file");
}

#[test]
fn an_unreadable_settings_file_is_read_as_off_rather_than_as_a_failure_to_start() {
    let path = scratch("corrupt");
    std::fs::write(&path, "{ this is not json").unwrap();
    let state = McpState::load_from(path.clone());
    assert!(!state.view().server.enabled);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_settings_file_that_predates_this_feature_stays_off() {
    // The realistic upgrade path: a file with a couple of unrelated keys.
    let path = scratch("older");
    std::fs::write(&path, r#"{"somethingElse": true}"#).unwrap();
    let state = McpState::load_from(path.clone());
    let view = state.view();
    assert!(!view.server.enabled);
    assert!(!view.client.enabled);
    assert_eq!(view.server.port, DEFAULT_PORT);
    let _ = std::fs::remove_file(path);
}

#[test]
fn settings_survive_a_restart_and_the_file_is_not_world_readable() {
    let path = scratch("persist");
    {
        let state = McpState::load_from(path.clone());
        state.stored.write().server.enabled = true;
        state.stored.write().server.token = Some(Token::from_raw("deadbeef"));
        state.stored.write().client.enabled = true;
        state.save().unwrap();
    }
    let reloaded = McpState::load_from(path.clone());
    assert!(reloaded.view().server.enabled);
    assert_eq!(reloaded.view().server.token_preview.as_deref(), Some("deadbe…"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the file holding the token was group/other readable");
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_view_names_every_tool_and_marks_exactly_the_writes() {
    // The pane's disclosure is rendered from this, so a tool added to the
    // catalogue appears in the sentence the user reads without anybody
    // remembering to update it.
    let state = McpState::load_from(scratch("tools"));
    let view = state.view();
    assert_eq!(view.tools.len(), wobu_mcp::catalogue().len());
    let writes: Vec<_> =
        view.tools.iter().filter(|tool| tool.write).map(|tool| tool.name.as_str()).collect();
    assert_eq!(writes, ["create_node", "update_node", "link_nodes"]);
    assert!(view.tools.iter().all(|tool| !tool.description.is_empty()));
}

#[test]
fn a_patch_touches_only_the_fields_it_names() {
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.summary = "A courier.".into();
    node.notes_raw = "handwritten".into();
    node.tags = vec!["draft".into()];
    node.attributes.insert("age".into(), json!(31));

    apply_patch(
        &mut node,
        &NodePatch { summary: Some("A smuggler.".into()), ..NodePatch::default() },
    );

    assert_eq!(node.summary, "A smuggler.");
    assert_eq!(node.name, "Kael", "a patch without a name renamed the node");
    assert_eq!(node.notes_raw, "handwritten");
    assert_eq!(node.tags, ["draft"]);
    assert_eq!(node.attributes["age"], json!(31));
}

#[test]
fn an_attribute_patch_merges_rather_than_replacing_the_map() {
    // An agent setting one fact should not drop the six somebody typed.
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.attributes.insert("age".into(), json!(31));
    node.attributes.insert("height".into(), json!("tall"));

    let mut attributes = serde_json::Map::new();
    attributes.insert("age".into(), json!(32));
    apply_patch(&mut node, &NodePatch { attributes: Some(attributes), ..NodePatch::default() });

    assert_eq!(node.attributes["age"], json!(32));
    assert_eq!(node.attributes["height"], json!("tall"));
}

#[test]
fn kinds_roles_and_ids_are_rejected_with_the_list_of_what_would_have_worked() {
    let error = parse_kind("wizard").unwrap_err();
    assert_eq!(error.code, Code::Invalid);
    assert!(error.message.contains("character"), "{}", error.message);

    let error = parse_role("befriends").unwrap_err();
    assert!(error.message.contains("related_to"), "{}", error.message);

    assert!(parse_id("not-a-ulid").is_err());
    assert!(parse_id("01ARZ3NDEKTSV4RRFFQ69G5FAV").is_ok());
}

#[test]
fn a_probe_refuses_a_server_that_is_not_enabled_without_launching_it() {
    // The predicate the probe command uses, pinned here because the command
    // itself needs a Tauri `State` to call.
    let mut settings = ClientSettings {
        enabled: true,
        servers: vec![ClientServer {
            id: "one".into(),
            name: "Notes".into(),
            command: "true".into(),
            enabled: false,
            ..ClientServer::default()
        }],
    };
    assert!(settings.active().next().is_none());
    settings.servers[0].enabled = true;
    assert!(settings.active().next().is_some());
    settings.enabled = false;
    assert!(settings.active().next().is_none());
}
