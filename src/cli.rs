use super::config::Config;
use super::database::Database;
use clap::{clap_app, crate_version, App, ArgMatches};

fn app() -> App<'static, 'static> {
    let app = clap_app!(cepler =>
        (version: crate_version!())
        (@setting VersionlessSubcommands)
        (@setting SubcommandRequiredElseHelp)
        (@arg CONFIG_FILE: -c --("config") env("CEPLER_CONF") default_value("cepler.yml") {config_file} "Cepler config file")
        (@arg STATE_FILE: -s --("state") env("CEPLER_STATE") default_value("cepler.state") "Cepler state file")
        (@subcommand record =>
            (about: "Record the state of an environment in the statefile")
            (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
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
        ("hook", Some(_)) => hook(conf_from_matches(&matches)),
        ("record", Some(sub_matches)) => record(
            sub_matches,
            matches.value_of("STATE_FILE").unwrap().to_string(),
            conf_from_matches(&matches),
        ),
        _ => unreachable!(),
    }
}

fn hook(conf: Config) {
    use std::process::{Command, Stdio};
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

fn record(matches: &ArgMatches, state_file: String, config: Config) {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    if let Some(env) = config.environments.get(env) {
        match Database::open(state_file) {
            Ok(mut db) => {
                if let Err(e) = db.record_env(env) {
                    println!("{}", e);
                }
            }
            Err(e) => {
                println!("{}", e);
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("Couldn't find environment '{}' in cepler.yml", env);
        std::process::exit(1);
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

fn conf_from_matches(matches: &ArgMatches) -> Config {
    Config::from_file(matches.value_of("CONFIG_FILE").unwrap()).unwrap()
}
