use super::{
    concourse::{self},
    config::*,
    database::Database,
    repo::*,
    workspace::Workspace,
};
use anyhow::*;
use clap::{clap_app, crate_version, App, ArgMatches};
use std::path::Path;

fn app() -> App<'static, 'static> {
    let app = clap_app!(cepler =>
        (version: crate_version!())
        (@setting VersionlessSubcommands)
        (@setting SubcommandRequiredElseHelp)
        (@arg CONFIG_FILE: -c --("config") env("CEPLER_CONF") default_value("cepler.yml") "Cepler config file")
        (@arg IGNORE_QUEUE: --("ignore-queue") "Ignore the propagation queue")
        (@arg GATES_FILE: -g --("gates") +takes_value env("CEPLER_GATES") "Cepler gate file")
        (@arg GATES_BRANCH: --("gates-branch") +takes_value requires_all(&["GATES_FILE"]) env("GATES_BRANCH") "Branch to find the gate file")
        (@arg CLONE_DIR: --("clone") +takes_value requires_all(&["GIT_URL", "GIT_PRIVATE_KEY"]) "Clone the repository into <dir>. Pulls latest changes if already present.")
        (@arg GIT_URL: --("git-url") +takes_value env("GIT_URL") "Remote url for --clone option")
        (@arg GIT_PRIVATE_KEY: --("git-private-key") +takes_value env("GIT_PRIVATE_KEY") "Private key for --clone option")
        (@arg GIT_BRANCH: --("git-branch") +takes_value default_value("main") env("GIT_BRANCH") "Branch for --clone option")
        (@subcommand check =>
          (about: "Check wether the environment needs deploying. Exit codes: 0 - needs deploying; 1 - internal error; 2 - nothing to deploy")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
        )
        (@subcommand ls =>
          (about: "List all files relevent to a given environment")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
        )
        (@subcommand latest =>
          (about: "Return the commit hash of the lastest record")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
        )
        (@subcommand record =>
          (about: "Record the state of an environment in the statefile")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
          (@arg NO_COMMIT: --("no-commit") "Don't commit the new state")
          (@arg RESET_HEAD: --("reset-head") "Checkout files to head after committing the state")
          (@arg PUSH: --("push") requires_all(&["RESET_HEAD", "GIT_URL", "GIT_PRIVATE_KEY"]) "Push head to remote")
          (@arg GIT_URL: --("git-url") +takes_value env("GIT_URL") "Remote url for --clone option")
          (@arg GIT_PRIVATE_KEY: --("git-private-key") +takes_value env("GIT_PRIVATE_KEY") "Private key for --clone option")
          (@arg GIT_BRANCH: --("git-branch") +takes_value default_value("main") env("GIT_BRANCH") "Branch for --clone option")
        )
        (@subcommand prepare =>
          (about: "Prepare workspace for hook execution")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
          (@arg FORCE_CLEAN: --("force-clean") "Delete all files not referenced in cepler.yml")
        )
        (@subcommand reproduce =>
          (about: "Reproduce workspace according to last recorded state")
          (@arg ENVIRONMENT: -e --("environment") env("CEPLER_ENVIRONMENT") +required +takes_value "The cepler environment")
          (@arg FORCE_CLEAN: --("force-clean") "Delete all files not referenced in cepler.yml")
        )
        (@subcommand concourse =>
         (@setting SubcommandRequiredElseHelp)
         (about: "Subcommand for concourse integration")
         (@subcommand check =>
          (about: "The check command for the concourse resource")
         )
         (@subcommand ci_in =>
          (about: "The in command for the concourse resource")
          (@arg DESTINATION: * "The destination to put the resource")
         )
         (@subcommand ci_out =>
          (about: "The out command for the concourse resource")
          (@arg ORIGIN: * "The destination to put the resource")
         )
      )
    );

    app
}

pub fn run() -> Result<()> {
    let matches = app().get_matches();
    let ignore_queue = matches.is_present("IGNORE_QUEUE");
    if let Some(dir) = matches.value_of("CLONE_DIR") {
        let conf = GitConfig {
            url: matches.value_of("GIT_URL").unwrap().to_string(),
            branch: matches.value_of("GIT_BRANCH").unwrap().to_string(),
            gates_branch: matches.value_of("GATES_BRANCH").map(|b| b.to_string()),
            private_key: matches.value_of("GIT_PRIVATE_KEY").unwrap().to_string(),
            dir: dir.to_string(),
        };
        let path = std::path::Path::new(&dir);
        if !path.exists() || path.read_dir()?.next().is_none() {
            Repo::clone(conf)?;
            std::env::set_current_dir(dir)?;
        } else {
            std::env::set_current_dir(dir)?;
            Repo::open(None)?.pull(conf)?;
        }
    }

    match matches.subcommand() {
        ("ls", Some(sub_matches)) => ls(
            sub_matches,
            conf_from_matches(&matches)?,
            gates_from_matches(&matches)?,
            ignore_queue,
        ),
        ("check", Some(sub_matches)) => check(
            sub_matches,
            conf_from_matches(&matches)?,
            gates_from_matches(&matches)?,
            ignore_queue,
        ),
        ("prepare", Some(sub_matches)) => prepare(
            sub_matches,
            conf_from_matches(&matches)?,
            gates_from_matches(&matches)?,
            ignore_queue,
        ),
        ("reproduce", Some(sub_matches)) => reproduce(sub_matches, conf_from_matches(&matches)?),
        ("record", Some(sub_matches)) => record(
            sub_matches,
            conf_from_matches(&matches)?,
            gates_from_matches(&matches)?,
            ignore_queue,
        ),
        ("latest", Some(sub_matches)) => latest(sub_matches, conf_from_matches(&matches)?),
        ("concourse", Some(sub_matches)) => match sub_matches.subcommand() {
            ("check", Some(_)) => concourse_check(),
            ("ci_in", Some(matches)) => concourse_in(matches),
            ("ci_out", Some(matches)) => concourse_out(matches),
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}

fn check(
    matches: &ArgMatches,
    (config, config_path): (Config, String),
    gates: Option<GatesConfig>,
    ignore_queue: bool,
) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let gate = if let Some(gates) = gates {
        gates.get_gate(env)?
    } else {
        None
    };
    let ws = Workspace::new(&config.scope, config_path.clone(), ignore_queue)?;
    let env = config.environments.get(env).context(format!(
        "Environment '{}' not found in config '{}'",
        env, config_path
    ))?;
    match ws.check(env, gate)? {
        None => {
            println!("Nothing new to deploy");
            std::process::exit(2);
        }
        Some((commit, _)) => {
            println!("Found new state to deploy - trigger commit {}", commit);
        }
    }
    Ok(())
}

fn ls(
    matches: &ArgMatches,
    (config, config_path): (Config, String),
    gates: Option<GatesConfig>,
    ignore_queue: bool,
) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let gate = if let Some(gates) = gates {
        gates.get_gate(env)?
    } else {
        None
    };
    let ws = Workspace::new(&config.scope, config_path.clone(), ignore_queue)?;
    let env = config.environments.get(env).context(format!(
        "Environment '{}' not found in config '{}'",
        env, config_path
    ))?;
    for path in ws.ls(env, gate)? {
        println!("{}", path);
    }
    Ok(())
}
fn prepare(
    matches: &ArgMatches,
    config: (Config, String),
    gates: Option<GatesConfig>,
    ignore_queue: bool,
) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let force_clean: bool = matches.is_present("FORCE_CLEAN");
    if force_clean {
        println!("WARNING removing all non-cepler specified files");
    }
    let gate = if let Some(gates) = gates {
        gates.get_gate(env)?
    } else {
        None
    };
    let env = config.0.environments.get(env).context(format!(
        "Environment '{}' not found in config '{}'",
        env, config.1
    ))?;
    let ws = Workspace::new(&config.0.scope, config.1, ignore_queue)?;
    ws.prepare(env, gate, force_clean)?;
    Ok(())
}
fn reproduce(matches: &ArgMatches, config: (Config, String)) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let force_clean: bool = matches.is_present("FORCE_CLEAN");
    if force_clean {
        println!("WARNING removing all non-cepler specified files");
    }
    let env = config.0.environments.get(env).context(format!(
        "Environment '{}' not found in config '{}'",
        env, config.1
    ))?;
    let ws = Workspace::new(&config.0.scope, config.1, false)?;
    ws.reproduce(env, force_clean)?;
    Ok(())
}

fn record(
    matches: &ArgMatches,
    config: (Config, String),
    gates: Option<GatesConfig>,
    ignore_queue: bool,
) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let gate = if let Some(gates) = gates {
        gates.get_gate(env)?
    } else {
        None
    };
    let commit = !matches.is_present("NO_COMMIT");
    let reset = matches.is_present("RESET_HEAD");
    let push = matches.is_present("PUSH");
    let git_config = if push {
        Some(GitConfig {
            url: matches.value_of("GIT_URL").unwrap().to_string(),
            branch: matches.value_of("GIT_BRANCH").unwrap().to_string(),
            gates_branch: None,
            private_key: matches.value_of("GIT_PRIVATE_KEY").unwrap().to_string(),
            dir: String::new(),
        })
    } else {
        None
    };
    let env = config.0.environments.get(env).context(format!(
        "Environment '{}' not found in config '{}'",
        env, config.1
    ))?;
    let mut ws = Workspace::new(&config.0.scope, config.1, ignore_queue)?;
    ws.record_env(env, gate, commit, reset, git_config)?;
    Ok(())
}

fn latest(matches: &ArgMatches, (config, config_file): (Config, String)) -> Result<()> {
    let env = matches.value_of("ENVIRONMENT").unwrap();
    let db = Database::open(&config.scope, &config_file, false)?;
    if let Some(env) = db.get_current_state(env) {
        println!("{}", env.head_commit.clone().inner());
    } else {
        eprintln!("Environment '{}' not deployed!", env);
        std::process::exit(1);
    }
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

#[allow(clippy::redundant_closure)]
fn gates_from_matches(matches: &ArgMatches) -> Result<Option<GatesConfig>> {
    let file_name = matches.value_of("GATES_FILE");
    if let Some(branch) = matches.value_of("GATES_BRANCH") {
        match Repo::open(None)?.get_file_from_branch(
            branch,
            Path::new(file_name.unwrap()),
            |bytes| GatesConfig::from_reader(bytes),
        ) {
            Ok(Some(config)) => Ok(Some(config)),
            Ok(_) => Err(anyhow!("Couldn't find gates file in branch")),
            err => err,
        }
    } else if let Some(f) = file_name {
        Ok(Some(GatesConfig::from_file(f)?))
    } else {
        Ok(None)
    }
}
