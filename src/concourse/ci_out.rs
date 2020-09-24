use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use std::{io, path};

pub fn exec(origin: &str) -> Result<()> {
    eprintln!("Recording resource - cepler v{}", clap::crate_version!());
    let ResourceConfig { source, params, .. }: ResourceConfig =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    std::env::set_current_dir(path::Path::new(&format!(
        "{}/{}",
        origin,
        params.unwrap().repository
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
    let (head, diff) = ws.record_env(env, true, true, Some(conf))?;
    println!(
        "{}",
        serde_json::to_string(&ResourceData {
            version: Version { head },
            metadata: diff
                .into_iter()
                .map(|diff| DiffElem {
                    name: diff.path,
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
