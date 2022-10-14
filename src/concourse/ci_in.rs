use super::*;
use crate::workspace::Workspace;
use glob::*;
use std::{io, path::Path};

pub fn exec(destination: &str) -> Result<()> {
    eprintln!("Preparing resource - cepler v{}", clap::crate_version!());
    let ResourceConfig {
        source,
        version,
        params,
    }: ResourceConfig = serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    let should_prepare = params.map(|p| p.prepare).unwrap_or(true);
    eprintln!("Cloning repo to '{}'", destination);
    let version = version.expect("No version specified");
    let conf = GitConfig {
        url: source.uri,
        branch: source.branch.clone(),
        gates_branch: source.gates_branch.clone(),
        private_key: source.private_key,
        dir: destination.to_string(),
    };

    let path = Path::new(&destination);
    let repo = Repo::clone(conf).context("Couldn't clone repo")?;
    std::env::set_current_dir(path)?;
    let (hash, summary) = repo.head_commit_summary()?;
    eprintln!(
        "HEAD of branch '{}' is now at: [{}] - {}",
        source.branch, hash, summary
    );

    let config = Config::from_file(&source.config)?;
    let ws = Workspace::new(&config.scope, source.config, source.ignore_queue)?;
    let environment = if let Some(environment) = source.environment {
        environment
    } else {
        eprintln!("No environment specified... providing an empty dir");
        return empty_repo(version);
    };
    let env = config
        .environments
        .get(&environment)
        .context(format!("Environment '{}' not found in config", environment))?;
    eprintln!(
        "Checking if we can prepare deployment at trigger '{}'",
        version.trigger
    );
    let wanted_trigger = &version.trigger;
    let gate = get_gate(
        source.gates_file.as_ref(),
        source.gates_branch.as_ref(),
        &environment,
        &repo,
    )?;

    let (state_id, diff) = if should_prepare {
        match ws.check(env, gate.clone())? {
            Some((state_id, _)) if &state_id.head_commit != wanted_trigger => {
                eprintln!("Trigger is out of sync.");
                std::process::exit(1);
            }
            None => {
                eprintln!("Nothing new to deploy... reproducing last state");
                let state_id = ws.reproduce(env, true)?;
                if &state_id.head_commit != wanted_trigger {
                    eprintln!("Reproduced state is out of sync - providing empty dir");
                    return empty_repo(version);
                }
                (state_id, Vec::new())
            }
            Some(ret) => {
                eprintln!("Preparing the workspace");
                ws.prepare(env, gate, true)?;
                ret
            }
        }
    } else {
        eprintln!("Reproducing last state");
        let state_id = ws.reproduce(env, true)?;
        if &state_id.head_commit != wanted_trigger {
            eprintln!("Reproduced state is out of sync - providing empty dir");
            return empty_repo(version);
        }
        (state_id, Vec::new())
    };

    std::fs::write(".git/cepler_environment", &environment)
        .context("Couldn't create file '.git/cepler_environment'")?;
    std::fs::write(".git/cepler_trigger", &state_id.head_commit)
        .context("Couldn't create file '.git/cepler_trigger'")?;

    println!(
        "{}",
        serde_json::to_string(&ResourceData {
            version,
            metadata: diff
                .into_iter()
                .map(|diff| DiffElem {
                    name: diff.ident.inner(),
                    value: diff
                        .current_state
                        .map(|state| state.to_string())
                        .unwrap_or_else(|| "File was removed".to_string())
                })
                .collect()
        })?
    );
    Ok(())
}

fn empty_repo(version: Version) -> Result<()> {
    for path in glob("*")? {
        let path = path?;
        if path.is_dir() {
            std::fs::remove_dir_all(path).context("Couldn't remove dir")?;
        } else {
            std::fs::remove_file(path).context("Couldn't remove file")?;
        }
    }
    println!(
        "{}",
        serde_json::to_string(&ResourceData {
            version,
            metadata: Vec::new()
        })?
    );
    Ok(())
}
