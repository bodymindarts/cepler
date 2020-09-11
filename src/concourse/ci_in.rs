use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use glob::*;
use std::{io, path};

pub fn exec(destination: &str) -> Result<()> {
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
    let env = config
        .environments
        .get(&source.environment)
        .context(format!(
            "Environment '{}' not found in config",
            source.environment
        ))?;
    eprintln!(
        "Checking if we can prepare deployment no '{}'",
        version.deployment_no
    );
    let wanted_no = version.deployment_no.parse()?;
    let wanted_head = &version.head;
    match ws.check(env)? {
        Some((n, head)) if n == wanted_no && &head == wanted_head => {
            eprintln!("Found new state to deploy");
        }
        Some((_, head)) if &head != wanted_head => eprintln!("Repo is out of date."),
        Some((n, _)) if n > wanted_no => {
            return Err(anyhow!(
                "Cannot provide resource. Last deployment was: '{}",
                n - 1
            ));
        }
        Some((n, _)) => {
            return Err(anyhow!(
                "Cannot provide resource. Next deployment would be: '{}",
                n
            ));
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
            return Ok(());
        }
    }
    eprintln!("Preparing the workspace");
    ws.prepare(env, true)?;

    println!("{}", serde_json::to_string(&InReturn { version })?);
    Ok(())
}
