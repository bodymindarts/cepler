use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use serde::{Deserialize, Serialize};
use std::{env, io, path};

const TMPDIR: &str = "TMPDIR";
pub fn exec() -> Result<()> {
    let Check { source, version }: Check =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    eprintln!(
        "Last deployment no: '{}', checking if we can deploy a newer version",
        version
            .as_ref()
            .map(|v| v.deployment_no.as_ref())
            .unwrap_or("0")
    );
    env::set_var(GIT_URL, source.uri);
    env::set_var(GIT_BRANCH, &source.branch);
    env::set_var(GIT_PRIVATE_KEY, source.private_key);
    let clone_dir = format!(
        "{}/cepler-repo-cache",
        env::var(TMPDIR).unwrap_or_else(|_| "/tmp".to_string())
    );
    let path = path::Path::new(&clone_dir);
    let repo = if !path.exists() || path.read_dir()?.next().is_none() {
        eprintln!("Cloning repo");
        let repo = Repo::clone(path).context("Couldn't clone repo")?;
        std::env::set_current_dir(path)?;
        repo
    } else {
        eprintln!("Pulling latest state");
        std::env::set_current_dir(path)?;
        let repo = Repo::open()?;
        repo.pull()?;
        repo
    };
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
    eprintln!("Checking equivalence with last deployed state...");
    let mut res = Vec::new();
    if let Some(version) = version {
        res.push(version);
    }
    match ws.check(env)? {
        None => {
            eprintln!("Nothing new to deploy");
        }
        Some(n) => {
            eprintln!("Found new state to deploy");
            res.push(Version {
                deployment_no: n.to_string(),
            })
        }
    }
    println!("{}", serde_json::to_string(&res)?);
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Check {
    source: Source,
    version: Option<Version>,
}
#[derive(Debug, Deserialize)]
struct Source {
    uri: String,
    branch: String,
    private_key: String,
    environment: String,
    #[serde(default = "default_config_path")]
    config: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct Version {
    deployment_no: String,
}

fn default_config_path() -> String {
    "cepler.yml".to_string()
}
