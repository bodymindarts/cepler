use super::{concourse::Concourse, config::Config, git::Repo, workspace::Workspace};
use anyhow::*;
use clap::{clap_app, crate_version, App, ArgMatches};

fn app() -> App<'static, 'static> {
    let app = clap_app!(cepler =>
        (version: crate_version!())
        (@setting VersionlessSubcommands)
        (@setting SubcommandRequiredElseHelp)
        (@arg CONFIG_FILE: -c --("config") env("CEPLER_CONF") default_value("cepler.yml") "Cepler config file")
        (@arg CLONE_DIR: -c --("clone") +takes_value "Clone the repository into <dir>")
        (@subcommand check =>
            (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
        )
        (@subcommand record =>
            (about: "Record the state of an environment in the statefile")
            (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
            (@arg NO_COMMIT: --("no-commit") "Don't commit the new state")
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

pub fn run() -> Result<()> {
    let matches = app().get_matches();
    if let Some(dir) = matches.value_of("CLONE_DIR") {
        let path = std::path::Path::new(&dir);
        if !path.exists() || path.read_dir()?.next().is_none() {
            Repo::clone(path)?;
            std::env::set_current_dir(dir)?;
        } else {
            std::env::set_current_dir(dir)?;
            Repo::open()?.pull()?;
        }
    }
    match matches.subcommand() {
        ("check", Some(sub_matches)) => check(sub_matches, &matches),
        ("prepare", Some(sub_matches)) => prepare(sub_matches, conf_from_matches(&matches)?),
        ("record", Some(sub_matches)) => record(sub_matches, conf_from_matches(&matches)?),
        ("concourse", Some(_)) => concourse(conf_from_matches(&matches)?),
        _ => unreachable!(),
    }
}

fn check(matches: &ArgMatches, main_matches: &ArgMatches) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let config = conf_from_matches(main_matches)?;
    let ws = Workspace::new(config.1)?;
    let env = config
        .0
        .environments
        .get(env)
        .context(format!("Environment '{}' not found in config", env))?;
    match ws.check(env) {
        Ok(deployment_no) => println!("{}", deployment_no),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    }
    Ok(())
}
fn prepare(matches: &ArgMatches, config: (Config, String)) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let force_clean: bool = matches.is_present("FORCE_CLEAN");
    if force_clean {
        println!("WARNING removing all non-cepler specified files");
    }
    let env = config
        .0
        .environments
        .get(env)
        .context(format!("Environment '{}' not found in config", env))?;
    let ws = Workspace::new(config.1)?;
    ws.prepare(env, force_clean)?;
    Ok(())
}

fn record(matches: &ArgMatches, config: (Config, String)) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let commit: bool = !matches.is_present("NO_COMMIT");
    let env = config
        .0
        .environments
        .get(env)
        .context(format!("Environment '{}' not found in config", env))?;
    let mut ws = Workspace::new(config.1)?;
    ws.record_env(env, commit)?;
    Ok(())
}

fn concourse((conf, _): (Config, String)) -> Result<()> {
    if conf.concourse.is_none() {
        return Err(anyhow!("concourse: key not specified"));
    }
    println!("{}", Concourse::new(conf).render_pipeline());
    Ok(())
}

fn conf_from_matches(matches: &ArgMatches) -> Result<(Config, String)> {
    let file_name = matches.value_of("CONFIG_FILE").unwrap();
    Ok((Config::from_file(file_name)?, file_name.to_string()))
}
