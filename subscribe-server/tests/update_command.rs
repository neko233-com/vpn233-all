include!("../../tests/support/update_command.rs");

#[test]
fn update_runs_platform_script_without_loading_configuration() {
    check_update_command(env!("CARGO_BIN_EXE_vpn233-subscribe-server"), true);
}

#[test]
fn update_propagates_script_failure() {
    check_update_command(env!("CARGO_BIN_EXE_vpn233-subscribe-server"), false);
}
