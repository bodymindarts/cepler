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
        (@subcommand hook =>
            (about: "Execute the hook")
        )
    );

    app
}

pub fn run() {
    let matches = app().get_matches();
    match matches.subcommand() {
        ("workspace", Some(matches)) => workspace(matches),
        ("hook", Some(_)) => hook(matches.value_of("CONFIG_FILE").unwrap()),
        _ => unreachable!(),
    }
}

fn workspace(_matches: &ArgMatches) {
    println!("Workspace")
}

fn hook(conf_file: &str) {
    use std::process::{Command, Stdio};
    let conf = Config::from_file(conf_file).unwrap();
    println!("Executing hook: '{}'", conf.hook.cmd);
    let mut cmd = Command::new(conf.hook.cmd)
        .args(&conf.hook.args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let status = cmd.wait().expect("Failed to run hook");
    match status.code() {
        Some(code) => println!("Exited with status code: '{}'", code),
        None => println!("Process terminated by signal"),
    }
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
