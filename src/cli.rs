use super::config::Config;
use clap::{clap_app, crate_version, App, ArgMatches};

fn app() -> App<'static, 'static> {
    let app = clap_app!(cepler =>
        (version: crate_version!())
        (@setting VersionlessSubcommands)
        (@setting SubcommandRequiredElseHelp)
        (@arg CONFIG_FILE: -c --("config") env("CEPLER_CONF") default_value("cepler.yml") {config_file} "Cepler config file")
        (@subcommand workspace =>
            (about: "Prepare workspace")
        )
    );

    app
}

pub fn run() {
    let matches = app().get_matches();
    match matches.subcommand() {
        ("workspace", Some(matches)) => workspace(matches),
        _ => unreachable!(),
    }
}

fn workspace(_matches: &ArgMatches) {
    println!("Workspace")
}

fn config_file(file: String) -> Result<(), String> {
    use std::path::Path;
    let path = Path::new(&file);
    match (path.exists(), path.is_file()) {
        (true, true) => Ok(()),
        (false, _) => Err(format!("File '{}' does not exist", file)),
        (_, false) => Err(format!("'{}' is not a file", file)),
    }?;
    Config::from_file(path).map_err(|e| format!("Couldn't parse config file - '{}'", e))?;
    Ok(())
}
