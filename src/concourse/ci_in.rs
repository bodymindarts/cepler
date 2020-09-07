use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
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
    match ws.check(env)? {
        Some(n) if n == wanted_no => {
            eprintln!("Found new state to deploy");
        }
        Some(n) if n > wanted_no => {
            return Err(anyhow!(
                "Cannot provide resource. Last deployment was: '{}",
                n - 1
            ));
        }
        Some(n) => {
            return Err(anyhow!(
                "Cannot provide resource. Next deployment would be: '{}",
                n
            ));
        }
        None => {
            return Err(anyhow!("Nothing new to deploy"));
        }
    }
    eprintln!("Preparing the workspace");
    ws.prepare(env, true)?;

    println!("{}", serde_json::to_string(&InReturn { version })?);
    Ok(())
}
