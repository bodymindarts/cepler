use super::{
    concourse::{self, ConcourseGen},
    config::Config,
    repo::Repo,
    workspace::Workspace,
};
use anyhow::*;
use clap::{clap_app, crate_version, App, ArgMatches};

fn app() -> App<'static, 'static> {
    let app = clap_app!(cepler =>
        (version: crate_version!())
        (@setting VersionlessSubcommands)
        (@setting SubcommandRequiredElseHelp)
        (@arg CONFIG_FILE: -c --("config") env("CEPLER_CONF") default_value("cepler.yml") "Cepler config file")
        (@arg CLONE_DIR: --("clone") +takes_value "Clone the repository into <dir>")
        (@subcommand check =>
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
        )
        (@subcommand record =>
          (about: "Record the state of an environment in the statefile")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
          (@arg NO_COMMIT: --("no-commit") "Don't commit the new state")
          (@arg RESET_HEAD: --("reset-head") "Checkout files to head after committing the state")
        )
        (@subcommand prepare =>
          (about: "Prepare workspace for hook execution")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
          (@arg FORCE_CLEAN: --("force-clean") "Delete all files not referenced in cepler.yml")
        )
        (@subcommand concourse =>
         (@setting SubcommandRequiredElseHelp)
         (about: "Subcommand for concourse integration")
         (@subcommand gen =>
          (about: "Generate a concourse pipeline")
         )
         (@subcommand check =>
          (about: "The check command for the concourse resource")
         )
         (@subcommand ci_in =>
          (about: "The in command for the concourse resource")
          (@arg DESTINATION: * "The destination to put the resource")
         )
         (@subcommand ci_out =>
          (about: "The in command for the concourse resource")
          (@arg ORIGIN: * "The destination to put the resource")
         )
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
        ("concourse", Some(sub_matches)) => match sub_matches.subcommand() {
            ("gen", Some(_)) => concourse_gen(conf_from_matches(&matches)?),
            ("check", Some(_)) => concourse_check(),
            ("ci_in", Some(matches)) => concourse_in(&matches),
            ("ci_out", Some(matches)) => concourse_out(&matches),
            _ => unreachable!(),
        },
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
    match ws.check(env)? {
        None => {
            println!("Nothing new to deploy");
            std::process::exit(2);
        }
        Some(_) => {
            println!("Found new state to deploy");
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
    let reset: bool = matches.is_present("RESET_HEAD");
    let env = config
        .0
        .environments
        .get(env)
        .context(format!("Environment '{}' not found in config", env))?;
    let mut ws = Workspace::new(config.1)?;
    ws.record_env(env, commit, reset)?;
    Ok(())
}

fn concourse_gen((conf, _): (Config, String)) -> Result<()> {
    if conf.concourse.is_none() {
        return Err(anyhow!("concourse: key not specified"));
    }
    println!("{}", ConcourseGen::new(conf).render_pipeline());
    Ok(())
}
fn concourse_check() -> Result<()> {
    concourse::check::exec()
}

fn concourse_in(matches: &ArgMatches) -> Result<()> {
    let destination = matches.value_of("DESTINATION").unwrap();
    concourse::ci_in::exec(destination)
}

fn concourse_out(matches: &ArgMatches) -> Result<()> {
    let origin = matches.value_of("ORIGIN").unwrap();
    concourse::ci_out::exec(origin)
}

fn conf_from_matches(matches: &ArgMatches) -> Result<(Config, String)> {
    let file_name = matches.value_of("CONFIG_FILE").unwrap();
    Ok((Config::from_file(file_name)?, file_name.to_string()))
}
