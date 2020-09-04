use super::{concourse::Concourse, config::Config, workspace::Workspace};
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
        (@subcommand prepare =>
            (about: "Prepare workspace for hook execution")
            (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
            (@arg FORCE_CLEAN: --("force-clean") "Delete all files not referenced in cepler.yml")
        )
        (@subcommand concourse =>
            (about: "Render a concourse pipeline")
        )
    );

    app
}

pub fn run() {
    let matches = app().get_matches();
    match matches.subcommand() {
        ("record", Some(sub_matches)) => record(
            sub_matches,
            matches.value_of("STATE_FILE").unwrap().to_string(),
            conf_from_matches(&matches),
        ),
        ("prepare", Some(sub_matches)) => prepare(
            sub_matches,
            matches.value_of("STATE_FILE").unwrap().to_string(),
            conf_from_matches(&matches),
        ),
        ("concourse", Some(_)) => concourse(conf_from_matches(&matches)),
        _ => unreachable!(),
    }
}

fn concourse((conf, _): (Config, String)) {
    if conf.concourse.is_none() {
        eprintln!("concourse: key not specified");
        std::process::exit(1);
    }
    println!("{}", Concourse::new(conf).render_pipeline())
}

fn record(matches: &ArgMatches, state_file: String, config: (Config, String)) {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    if let Some(env) = config.0.environments.get(env) {
        match Workspace::new(state_file, config.1) {
            Ok(mut ws) => {
                if let Err(e) = ws.record_env(env) {
                    println!("{}", e);
                } else {
                    println!("State of '{}' recorded", env.name);
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

fn prepare(matches: &ArgMatches, state_file: String, config: (Config, String)) {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let force_clean: bool = matches.is_present("FORCE_CLEAN");
    if force_clean {
        println!("WARNING removing all non-cepler specified files");
    }
    if let Some(env) = config.0.environments.get(env) {
        match Workspace::new(state_file, config.1) {
            Ok(ws) => {
                if let Err(e) = ws.prepare(env, force_clean) {
                    println!("{}", e);
                } else {
                    println!("Workspace prepared to deploy '{}'", env.name);
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

fn conf_from_matches(matches: &ArgMatches) -> (Config, String) {
    let file_name = matches.value_of("CONFIG_FILE").unwrap();
    (Config::from_file(file_name).unwrap(), file_name.to_string())
}
