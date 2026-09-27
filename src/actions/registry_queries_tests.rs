use super::*;

#[test]
fn kind_queries_include_infra_and_only_matching_curated_actions() {
    let sonarr = valid_actions_for_kind(ServiceKind::Sonarr);
    assert!(sonarr.contains(&"help"));
    assert!(sonarr.contains(&"op"));
    assert!(!sonarr.contains(&"download_queue"));
    assert!(action_allowed_for_kind("help", ServiceKind::Sonarr));
    assert!(!action_allowed_for_kind(
        "download_queue",
        ServiceKind::Sonarr
    ));
}

#[test]
fn curated_parameter_queries_are_consistent() {
    assert!(curated_param_names().contains(&"service"));
    for action in actions_for_curated_param("service") {
        assert!(
            required_params_for_action(action).contains(&"service")
                || curated_command(action)
                    .is_some_and(|command| command.optional_params.contains(&"service"))
        );
    }
}

#[test]
fn qbittorrent_controls_are_not_advertised_or_allowed_for_sabnzbd() {
    let qbit = valid_actions_for_kind(ServiceKind::Qbittorrent);
    let sab = valid_actions_for_kind(ServiceKind::Sabnzbd);
    for action in crate::actions::commands::download::QBITTORRENT_ONLY_COMMANDS {
        assert!(qbit.contains(action), "qBittorrent missing {action}");
        assert!(!sab.contains(action), "SABnzbd advertised {action}");
        assert!(action_allowed_for_kind(action, ServiceKind::Qbittorrent));
        assert!(!action_allowed_for_kind(action, ServiceKind::Sabnzbd));
        assert_eq!(allowed_kind_names_for_action(action), vec!["qbittorrent"]);
    }
}
