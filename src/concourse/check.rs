use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use std::{
    env,
    fs::File,
    io::{self, Write},
    path,
};

const TMPDIR: &str = "TMPDIR";
pub fn exec() -> Result<()> {
    let resource: ResourceConfig =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    let ResourceConfig {
        source, version, ..
    }: ResourceConfig = resource.clone();
    if let Some(ref version) = version {
        eprintln!(
            "Last deployed head: '{}', checking if we can deploy a newer version",
            version.head
        );
    } else {
        eprintln!("No previous deployments - checking if there is one");
    }

    let clone_dir = format!(
        "{}/cepler-repo-cache",
        env::var(TMPDIR).unwrap_or_else(|_| "/tmp".to_string())
    );
    let mut file = File::create(&format!(
        "{}/cepler-check-input",
        env::var(TMPDIR).unwrap_or_else(|_| "/tmp".to_string())
    ))?;
    file.write_all(&serde_json::to_vec(&resource)?)?;
    let conf = GitConfig {
        url: source.uri,
        branch: source.branch.clone(),
        private_key: source.private_key,
        dir: clone_dir.clone(),
    };
    let path = path::Path::new(&clone_dir);
    let repo = if !path.exists() || path.read_dir()?.next().is_none() {
        eprintln!("Cloning repo");
        let repo = Repo::clone(conf).context("Couldn't clone repo")?;
        std::env::set_current_dir(path)?;
        repo
    } else {
        eprintln!("Pulling latest state");
        std::env::set_current_dir(path)?;
        let repo = Repo::open()?;
        repo.pull(conf)?;
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
    match (version, ws.check(env)?) {
        (None, Some((head, _))) => {
            eprintln!("Found new state to deploy");
            res.push(Version { head })
        }
        (Some(last), Some((head, _))) if last.head != head => {
            eprintln!("Found new state to deploy");
            res.push(last);
            res.push(Version { head })
        }
        (Some(last), _) => {
            eprintln!("Nothing new to deploy");
            res.push(last);
        }
        _ => {
            eprintln!("Nothing new to deploy");
        }
    }
    println!("{}", serde_json::to_string(&res)?);
    Ok(())
}
