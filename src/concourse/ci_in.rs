use super::*;
use crate::workspace::Workspace;
use glob::*;
use std::{fs, io, path::Path};

pub fn exec(destination: &str) -> Result<()> {
    eprintln!("Preparing resource - cepler v{}", clap::crate_version!());
    let ResourceConfig {
        source,
        version,
        params,
    }: ResourceConfig<InParams> =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    let should_prepare = params.map(|p| p.prepare).unwrap_or(true);
    let version = version.expect("No version specified");

    // Short-circuit when no environment is configured: the cloned repo would
    // only be deleted by `empty_repo` anyway, so skip the expensive clone.
    let Some(environment) = source.environment.clone() else {
        eprintln!("No environment specified... providing an empty dir");
        fs::create_dir_all(destination)?;
        std::env::set_current_dir(destination)?;
        return empty_repo(version);
    };

    eprintln!("Cloning repo to '{}'", destination);
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

    // `Repo::clone` skips the implicit full-tree checkout; pull in just
    // the files cepler reads from disk before continuing.
    let config = populate_workspace_metadata(
        &repo,
        &source.config,
        source.gates_file.as_ref(),
        source.gates_branch.as_ref(),
    )?;
    let ws = Workspace::new(&config.scope, source.config, source.ignore_queue)?;
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
        let t = std::time::Instant::now();
        let check_result = ws.check(env, gate.clone())?;
        eprintln!(
            "[cepler-perf] ws.check (construct_env_state walk): {:.2}s",
            t.elapsed().as_secs_f64()
        );
        match check_result {
            Some((state_id, _)) if &state_id.head_commit != wanted_trigger => {
                eprintln!("Trigger is out of sync.");
                std::process::exit(1);
            }
            None => {
                eprintln!("Nothing new to deploy... reproducing last state");
                let t = std::time::Instant::now();
                let state_id = ws.reproduce(env, true)?;
                eprintln!(
                    "[cepler-perf] ws.reproduce: {:.2}s",
                    t.elapsed().as_secs_f64()
                );
                if &state_id.head_commit != wanted_trigger {
                    eprintln!("Reproduced state is out of sync - providing empty dir");
                    return empty_repo(version);
                }
                (state_id, Vec::new())
            }
            Some(ret) => {
                eprintln!("Preparing the workspace");
                let t = std::time::Instant::now();
                ws.prepare(env, gate, true)?;
                eprintln!(
                    "[cepler-perf] ws.prepare (construct_env_state + propagated checkouts): {:.2}s",
                    t.elapsed().as_secs_f64()
                );
                ret
            }
        }
    } else {
        eprintln!("Reproducing last state");
        let t = std::time::Instant::now();
        let state_id = ws.reproduce(env, true)?;
        eprintln!(
            "[cepler-perf] ws.reproduce: {:.2}s",
            t.elapsed().as_secs_f64()
        );
        if &state_id.head_commit != wanted_trigger {
            eprintln!("Reproduced state is out of sync - providing empty dir");
            return empty_repo(version);
        }
        (state_id, Vec::new())
    };

    std::fs::write(".git/cepler_environment", &environment)
        .context("Couldn't create file '.git/cepler_environment'")?;
    std::fs::write(".git/cepler_trigger", state_id.head_commit)
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
