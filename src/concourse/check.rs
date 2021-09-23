use super::*;
use crate::{config::Config, workspace::Workspace};
use std::{
    env,
    fs::File,
    io::{self, Write},
    path,
};

const TMPDIR: &str = "TMPDIR";
pub fn exec() -> Result<()> {
    eprintln!(
        "Checking for new resource - cepler v{}",
        clap::crate_version!()
    );
    let resource: ResourceConfig =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    let ResourceConfig {
        source, version, ..
    }: ResourceConfig = resource.clone();
    if let Some(ref version) = version {
        eprintln!(
            "Last trigger: '{}', checking if we can deploy a newer version",
            version.trigger
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
        gates_branch: source.gates_branch.clone(),
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
        let repo = Repo::open(None)?;
        repo.pull(conf)?;
        repo
    };
    let (hash, summary) = repo.head_commit_summary()?;
    eprintln!(
        "HEAD of branch '{}' is now at: [{}] - {}",
        source.branch, hash, summary
    );

    let config = Config::from_file(&source.config)?;
    let ws = Workspace::new(&config.scope, source.config)?;
    let environment = source
        .environment
        .ok_or_else(|| anyhow!("Environment not specified in source"))?;
    let env = config
        .environments
        .get(&environment)
        .context(format!("Environment '{}' not found in config", environment))?;
    eprintln!("Checking equivalence with last deployed state...");
    let mut res = Vec::new();
    let gate = get_gate(
        source.gates_file.as_ref(),
        source.gates_branch.as_ref(),
        &environment,
        &repo,
    )?;
    match (version, ws.check(env, gate)?) {
        (None, Some((trigger, _))) => {
            eprintln!("Found new state to deploy");
            res.push(Version { trigger })
        }
        (Some(last), Some((trigger, _))) if last.trigger != trigger => {
            eprintln!("Found new state to deploy");
            res.push(last);
            res.push(Version { trigger })
        }
        (Some(last), ret) => {
            match ret {
                Some((trigger, _)) if last.trigger == trigger => {
                    eprintln!("Last trigger is still up to date")
                }
                _ => eprintln!("Nothing new to deploy"),
            }
            res.push(last);
        }
        _ => {
            eprintln!("Nothing new to deploy");
        }
    }
    println!("{}", serde_json::to_string(&res)?);
    Ok(())
}
