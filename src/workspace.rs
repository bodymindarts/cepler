use super::{config::EnvironmentConfig, database::*, repo::*};
use anyhow::*;

pub struct Workspace {
    path_to_config: String,
    db: Database,
}
impl Workspace {
    pub fn new(path_to_config: String) -> Result<Self> {
        Ok(Self {
            db: Database::open(&path_to_config)?,
            path_to_config,
        })
    }

    pub fn check(&self, env: &EnvironmentConfig) -> Result<Option<(usize, String)>> {
        let repo = Repo::open()?;
        if let Some(previous_env) = env.propagated_from() {
            self.db.get_current_state(&previous_env).context(format!(
                "Previous environment '{}' not deployed yet",
                previous_env
            ))?;
        }
        let new_env_state = self.construct_env_state(&repo, env, false)?;
        if let Some((last, deployment_no)) = self.db.get_current_state(&env.name) {
            return if last.equivalent(&new_env_state) {
                Ok(None)
            } else {
                Ok(Some((
                    deployment_no,
                    new_env_state.head_commit.to_short_ref(),
                )))
            };
        }
        Ok(Some((1, new_env_state.head_commit.to_short_ref())))
    }

    pub fn prepare(&self, env: &EnvironmentConfig, force_clean: bool) -> Result<()> {
        let repo = Repo::open()?;
        let head_files = if force_clean {
            Some(env.head_filters())
        } else {
            None
        };
        let ignore_list = vec![self.path_to_config.as_ref(), self.db.state_dir.as_ref()];
        repo.checkout_head(head_files, ignore_list)?;
        for file in env.propagated_files() {
            std::fs::remove_file(file).expect("Couldn't remove file");
        }
        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = self.db.get_target_propagated_state(&env.name, previous_env) {
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, state) in env_state.files.iter() {
                    if patterns.iter().any(|p| p.matches(&name)) {
                        repo.checkout_file_from(name, &state.from_commit)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn record_env(
        &mut self,
        env: &EnvironmentConfig,
        commit: bool,
        reset: bool,
        git_config: Option<GitConfig>,
    ) -> Result<(usize, String)> {
        eprintln!("Recording current state");
        let repo = Repo::open()?;
        let new_env_state = self.construct_env_state(&repo, env, true)?;
        let short_ref = new_env_state.head_commit.to_short_ref();
        let (state_file, deployment) = self
            .db
            .set_current_environment_state(env.name.clone(), new_env_state)?;
        if commit {
            eprintln!("Adding commit to repository to persist state");
            repo.commit_state_file(state_file)?;
        }
        if reset {
            eprintln!("Reseting head to have a clean workspace");
            repo.checkout_head(None, Vec::new())?;
        }
        if let Some(config) = git_config {
            eprintln!("Pushing to remote");
            repo.push(config)?;
        }
        Ok((deployment + 1, short_ref))
    }

    fn construct_env_state(
        &self,
        repo: &Repo,
        env: &EnvironmentConfig,
        recording: bool,
    ) -> Result<DeployState> {
        let head_commit = repo.head_commit_hash()?;
        let mut new_env_state = DeployState::new(head_commit);

        for file in repo.head_files(env.head_filters()) {
            let dirty = repo.is_file_dirty(&file)?;
            let file_name = file.to_str().unwrap().to_string();
            let (from_commit, message) = repo.find_last_changed_commit(&file)?;
            let file_hash = hash_file(file);
            let state = FileState {
                file_hash,
                dirty,
                from_commit,
                message,
            };
            new_env_state.files.insert(file_name, state);
        }

        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = self.db.get_target_propagated_state(&env.name, previous_env) {
                new_env_state.propagated_head = Some(env_state.head_commit.clone());
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, prev_state) in env_state.files.iter() {
                    if patterns.iter().any(|p| p.matches(&name)) {
                        let (dirty, file_hash) = if recording {
                            let file_hash = hash_file(name);
                            (file_hash != prev_state.file_hash, file_hash)
                        } else {
                            (false, prev_state.file_hash.clone())
                        };
                        let file_state = FileState {
                            dirty,
                            file_hash,
                            from_commit: prev_state.from_commit.clone(),
                            message: prev_state.message.clone(),
                        };
                        new_env_state.files.insert(name.clone(), file_state);
                    }
                }
            }
        }

        Ok(new_env_state)
    }
}
