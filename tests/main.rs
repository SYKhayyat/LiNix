//! **One test binary.**
//!
//! `tests/` held 101 files and cargo auto-discovers each as its own target, so every one of them
//! was fat-LTO-linked against a 100k-line crate under `codegen-units = 1` — and 36 of them never
//! call the library API at all, only spawning `CARGO_BIN_EXE_shall`. The `target/` directory that
//! produced reached **194 GB** and filled a 944 GB disk mid-build.
//!
//! **The suite already paid this cost once and wrote it down.** `mock_providers/mod.rs` opens by
//! recording that a top-level `mock_providers.rs` became *"a 716 KB binary containing zero
//! tests… compiled nineteen times"*, and that the fix was to make it a directory. This is the
//! same fix at the scale of the whole directory: one target, ninety-nine link units saved, and
//! `mock_providers` compiled once rather than nineteen times.
//!
//! **The conversion deleted nothing.** `autotests = false` in `Cargo.toml` is what stops cargo
//! claiming each file as a target, and the list below is what replaces the claim. A file added
//! to `tests/` and not listed here does not run — the one cost of this arrangement, and why
//! `every_test_file_is_in_the_suite` at the bottom fails when the two disagree.
//!
//! Four files were later renamed into the directory's filename-as-a-sentence convention, out of
//! the pre-v7 `test_*_wiring` style they were the last of; one test was deleted, by name, in
//! favour of the successor whose own header cites it as the gap it exists to close.
//!
//! Two modules carry a `cfg`: their files opened with an inner `#![cfg(...)]`, which cannot
//! survive becoming a module. On the `mod` line it is strictly better — off-platform the module
//! is not compiled at all rather than compiled to nothing.

/// The shared test doubles. Declared once here, which is the whole point: nineteen files said
/// `mod mock_providers;` and each got its own copy.
mod mock_providers;

/// The `Fixture` sixteen files wrote out by hand, three of them differently — see the module's
/// own header for what the three ways were and why the union is the correct one.
mod harness;

/// The four assertions every scanning gate makes about its own exemption table. Nine files
/// wrote them out by hand and three of the nine were missing the one that matters.
mod ledger;

mod a_backend_is_a_row_tests;
mod a_batch_of_installs_is_one_command_tests;
mod a_config_travels_between_machines_tests;
mod a_configured_capability_is_a_registered_one_tests;
mod a_downloaded_artifact_is_named_by_its_key_tests;
mod a_failed_sync_fails_under_every_flag_tests;
mod a_firewall_teardown_is_a_removal_tests;
mod a_lister_cannot_report_what_was_removed_tests;
mod a_machine_converges_tests;
mod a_module_is_a_subject_not_a_pile_tests;
mod a_parser_can_say_it_did_not_understand_tests;
mod a_plan_installs_only_declarations_tests;
mod a_plan_reaps_only_what_it_was_asked_about_tests;
mod a_pub_fn_nobody_calls_is_dead_tests;
mod a_shim_runs_what_its_line_named_tests;
mod a_silenced_advisory_says_why_tests;
mod a_spawned_child_has_an_owner_tests;
mod a_version_pin_is_honoured_or_explained_tests;
mod a_writer_that_reaches_the_disk_goes_through_one_tests;
mod absent_marker_coverage_tests;
mod adapter_tables_share_one_mechanism_tests;
mod an_ephemeral_shell_leaves_nothing_behind_tests;
mod an_exemption_table_is_audited_the_same_way_tests;
mod an_extension_surface_has_a_front_door_tests;
mod an_option_list_survives_the_seam_tests;
mod an_orphan_of_a_killed_run_is_taken_back_tests;
mod ansi_is_for_terminals_tests;
mod argv_drift_tests;
mod automation_lifecycle_tests;
mod backend_count_matches_the_spec_tests;
mod backend_is_data_not_code_tests;
mod backend_tests;
mod benign_exit_contradiction_tests;
mod byte_order_mark_tests;
mod conda_is_a_data_row_tests;
mod config_root_is_absolute_tests;
mod critical_paths_tests;
mod declared_options_converge_tests;
mod deploy_refusal_precedes_the_download_tests;
mod dotfiles_tree_is_a_pile_of_links_tests;
mod dry_run_every_verb_tests;
mod dry_run_marker_tests;
mod dry_run_tests;
mod every_example_is_checked_tests;
mod exec_lifecycle_tests;
mod exit_code_contract_tests;
mod failure_class_line_tests;
mod fanout_cap_reads_the_setting_tests;
mod feature_logic_tests;
mod first_hour_tests;
mod grade2_check_extras_tests;
mod grade2_flag_drift_blindspot_tests;
mod grade2_info_tests;
mod grade2_plan_extras_tests;
mod grade2_wedged_manifest_tests;
mod grade3_dry_run_data_dir_tests;
mod grade3_pixi_list_fixture_tests;
mod grade3_plan_guard_kind_tests;
mod grade3_protected_inspector_tests;
mod grade3_resource_idempotency_tests;
mod grade4_adopt_respects_the_manifest_tests;
mod grade4_heal_reports_its_own_failure_tests;
mod grade4_keyword_is_not_a_package_tests;
mod grade4_refusal_names_the_line_tests;
mod grade5_adopt_takes_services_tests;
mod grade6_backslash_is_not_set_math_tests;
mod grade6_binary_reachable_oracle_tests;
mod grade6_gate_parity_sees_whole_jobs_tests;
mod grade6_option_edit_reaches_the_machine_tests;
mod grade7_protected_skip_is_reported_tests;
mod grader_dry_run_siblings_tests;
mod grader_extras_guard_tests;
mod grader_gate_parity_tests;
mod grader_refusal_exit_code_tests;
#[cfg(windows)]
mod grader_shim_exit_code_tests;
mod grader_transient_claim_tests;
mod grader_unknown_backend_tests;
mod grammar_table_matches_the_spec_tests;
mod hardening_tests;
mod helm_verify_flag_tests;
mod help_map_tests;
mod hook_reentrancy_tests;
mod id_namespaces_do_not_collide_tests;
mod installed_listing_fixture_tests;
mod json_output_is_a_document_tests;
mod latency_budget_tests;
mod ledger_file_rules_tests;
mod lifecycle_coverage_union_tests;
mod lock_default_tests;
mod lock_scope_tests;
mod lock_unlock_axis_tests;
mod named_commands_exist_tests;
mod one_parser_reads_a_removal_target_tests;
mod option_table_coverage_tests;
mod os_native_argv_coverage_tests;
mod output_is_sanitized_tests;
mod parser_fixture_tests;
mod phase_is_the_sync_order_tests;
mod planner_scope_enumeration_tests;
mod priority_gates_every_fan_out_tests;
mod prompt_guard_tests;
#[cfg(target_os = "linux")]
mod pty_tests;
mod recovery_finishes_what_it_can_tests;
mod removal_guard_enumeration_tests;
mod resource_plan_family_tests;
mod security_and_resiliency_tests;
mod snapshot_restore_reaches_every_provider_tests;
mod startup_budget_tests;
mod terminator_probe_tests;
mod the_engine_runs_the_graph_in_order_tests;
mod the_kernel_assembles_what_it_was_configured_with_tests;
mod the_log_covers_what_cannot_be_recomputed_tests;
mod the_review_apparatus_is_rust_tests;
mod time_travel_tests;
mod unknown_backend_family_tests;
mod verbs_are_reachable_tests;
mod wal_enumeration_tests;
mod why_entries_are_attached_to_something_tests;

/// **A file in `tests/` that is not a module here never runs.**
///
/// That is the price of `autotests = false`, and it is payable only because it is checked. A
/// suite that silently stops running a file is worse than one that takes ninety-nine link units.
#[test]
fn every_test_file_is_in_the_suite() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let declared: std::collections::BTreeSet<String> = std::fs::read_to_string(dir.join("main.rs"))
        .expect("tests/main.rs is readable")
        .lines()
        .filter_map(|l| l.trim().strip_prefix("mod ")?.strip_suffix(';'))
        .map(str::to_string)
        .collect();

    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(&dir)
        .expect("tests/ is readable")
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".rs").map(str::to_string)
        })
        .filter(|n| n != "main")
        .collect();

    let missing: Vec<&String> = on_disk.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "these files are in tests/ and are not modules of the suite, so nothing runs them: \
         {missing:?}\nAdd `mod <name>;` to tests/main.rs."
    );

    // And the other direction: a `mod` naming a file that is gone does not compile, so the only
    // thing left to check is that the scan found the suite at all.
    assert!(
        declared.len() > 90,
        "only {} module(s) found in tests/main.rs; this scan has stopped matching it",
        declared.len()
    );
}
