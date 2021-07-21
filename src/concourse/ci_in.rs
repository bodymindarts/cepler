use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use glob::*;
use std::{io, path};

pub fn exec(destination: &str) -> Result<()> {
    eprintln!("Preparing resource - cepler v{}", clap::crate_version!());
    let ResourceConfig {
        source, version, ..
    }: ResourceConfig = serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    eprintln!("Cloning repo to '{}'", destination);
    let version = version.expect("No version specified");
    let conf = GitConfig {
        url: source.uri,
        branch: source.branch.clone(),
        private_key: source.private_key,
        dir: destination.to_string(),
    };

    let path = path::Path::new(&destination);
    let repo = Repo::clone(conf).context("Couldn't clone repo")?;
    std::env::set_current_dir(path)?;
    eprintln!(
        "HEAD of branch '{}' is now at: '{}'",
        source.branch,
        repo.head_commit_hash()?
    );

    let config = Config::from_file(&source.config)?;
    let ws = Workspace::new(source.config)?;
    let environment = source
        .environment
        .ok_or(anyhow!("Environment not specified in source"))?;
    let env = config
        .environments
        .get(&environment)
        .context(format!("Environment '{}' not found in config", environment))?;
    eprintln!(
        "Checking if we can prepare deployment at head '{}'",
        version.head
    );
    let wanted_head = &version.head;
    let diff = match ws.check(env)? {
        Some((head, diff)) if &head != wanted_head => {
            eprintln!("Repo is out of date.");
            diff
        }
        None => {
            eprintln!("Nothing new to deploy... providing an empty dir");
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
            return Ok(());
        }
        Some((_, diff)) => diff,
    };
    eprintln!("Preparing the workspace");
    ws.prepare(env, true)?;

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
                        .unwrap_or_else(String::new)
                })
                .collect()
        })?
    );
    Ok(())
}
