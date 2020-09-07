use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use std::{io, path};

pub fn exec(origin: &str) -> Result<()> {
    let ResourceConfig { source, params, .. }: ResourceConfig =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    std::env::set_current_dir(path::Path::new(&format!(
        "{}/{}",
        origin,
        params.unwrap().repo
    )))?;

    let conf = GitConfig {
        url: source.uri,
        branch: source.branch.clone(),
        private_key: source.private_key,
        dir: origin.to_string(),
    };
    let config = Config::from_file(&source.config)?;
    let mut ws = Workspace::new(source.config)?;
    let env = config
        .environments
        .get(&source.environment)
        .context(format!(
            "Environment '{}' not found in config",
            source.environment
        ))?;
    let deployment = ws.record_env(env, true, true, Some(conf))?;
    println!(
        "{}",
        serde_json::to_string(&InReturn {
            version: Version {
                deployment_no: deployment.to_string()
            }
        })?
    );
    Ok(())
}
