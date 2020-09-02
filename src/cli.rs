use clap::{clap_app, crate_version, App, ArgMatches};
use env_logger::Env;
use log::Level;
use std::{collections::HashMap, env, path::PathBuf, str::FromStr};

fn app() -> App<'static, 'static> {
    let app = clap_app!(cepler =>
    (version: crate_version!())
    // (@setting VersionlessSubcommands)
    // (@setting SubcommandRequiredElseHelp)
    // (@subcommand daemon =>
    //  (about: "Runs the risq p2p node")
    //  (visible_alias: "d")
    //  (@arg API_PORT: --("api-port") default_value("7477") {port} "API port")
    //  (@arg LOG_LEVEL: -l --("log-level") default_value("info") {level} "(error|warn|info|debug|trace)")
    //  (@arg NETWORK: -n --network default_value("BtcMainnet") {network} "(BtcRegtest|BtcTestnet|BtcMainnet)")
    //  (@arg P2P_PORT: -p --("p2p-port") default_value("5000") {port} "Port of p2p node")
    //  (@arg FORCE_SEED: --("force-seed") +takes_value {node_address} "Force usage of seed node")
    //  (@arg NO_TOR: --("no-tor") "Disable tor / run on localhost")
    //  (@arg TOR_CONTROL_PORT: --("tor-control-port") default_value("9051") {port} "Tor Control port")
    //  (@arg TOR_HIDDEN_SERVICE_PORT: --("tor-hidden-service-port") default_value("9999") {port} "Public port of the hidden service")
    //  (@arg TOR_SOCKS_PORT: --("tor-socks-port") default_value("9050") {port} "Tor SOCKSPort")
    // )
    // (@subcommand offers =>
    //  (about: "Subcommand to interact with offers")
    //  (@arg API_PORT: --("api-port") default_value("7477") {port} "API port")
    //  (@arg MARKET: --("market") default_value("all") {market} "Filter by market pair")
    // )
    );

    app
}

pub fn run() {
    let matches = app().get_matches();
    match matches.subcommand() {
        _ => unreachable!(),
    }
}
