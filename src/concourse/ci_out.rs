use super::*;
use crate::{config::Config, workspace::Workspace};
use std::{io, path};

pub fn exec(origin: &str) -> Result<()> {
    eprintln!("Recording resource - cepler v{}", clap::crate_version!());
    let ResourceConfig { source, params, .. }: ResourceConfig<OutParams> =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    let out_params = params.unwrap();
    std::env::set_current_dir(path::Path::new(&format!(
        "{}/{}",
        origin, out_params.repository
    )))?;

    let conf = GitConfig {
        url: source.uri,
        branch: source.branch.clone(),
        gates_branch: source.gates_branch.clone(),
        private_key: source.private_key,
        dir: origin.to_string(),
    };
    let config = Config::from_file(&source.config)?;
    let environment = out_params.environment.ok_or(()).or_else(|_| {
        source
            .environment
            .ok_or_else(|| anyhow!("Environment not specified in source"))
    })?;
    let mut ws = Workspace::new(&config.scope, source.config, source.ignore_queue)?;
    let env = config
        .environments
        .get(&environment)
        .context(format!("Environment '{}' not found in config", environment))?;
    let gate = get_gate(
        source.gates_file.as_ref(),
        source.gates_branch.as_ref(),
        &environment,
        &Repo::open(None)?,
    )?;
    let (state_id, diff) = ws.record_env(env, gate, true, true, Some(conf))?;
    println!(
        "{}",
        serde_json::to_string(&ResourceData {
            version: Version::from(state_id),
            metadata: diff
                .into_iter()
                .map(|diff| DiffElem {
                    name: diff.ident.inner(),
                    value: diff
                        .current_state
                        .map(|state| state.to_string())
                        .unwrap_or_default()
                })
                .collect()
        })?
    );
    Ok(())
}
