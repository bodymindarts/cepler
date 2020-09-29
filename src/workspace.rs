use super::{config::*, database::*, repo::*};
use anyhow::*;
use std::path::Path;

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

    pub fn ls(&self, env: &EnvironmentConfig) -> Result<Vec<String>> {
        let repo = Repo::open()?;
        let new_env_state = self.construct_env_state(&repo, &env, false)?;
        Ok(new_env_state.files.into_iter().map(|(k, _)| k).collect())
    }

    pub fn check(&self, env: &EnvironmentConfig) -> Result<Option<(String, Vec<FileDiff>)>> {
        let repo = Repo::open()?;
        if let Some(previous_env) = env.propagated_from() {
            self.db.get_current_state(&previous_env).context(format!(
                "Previous environment '{}' not deployed yet",
                previous_env
            ))?;
        }
        let new_env_state = self.construct_env_state(&repo, &env, false)?;
        let diffs = if let Some(last) = self.db.get_current_state(&env.name) {
            let diffs = new_env_state.diff(&last);
            if diffs.is_empty() {
                return Ok(None);
            }
            diffs
        } else {
            new_env_state
                .files
                .iter()
                .map(|(path, state)| FileDiff {
                    path: path.clone(),
                    current_state: Some(state.clone()),
                    added: true,
                })
                .collect()
        };
        for diff in diffs.iter() {
            if diff.added {
                eprintln!("File {} was added", diff.path)
            } else if diff.current_state.is_some() {
                eprintln!("File {} changed", diff.path)
            } else {
                eprintln!("File {} was removed", diff.path)
            }
        }
        Ok(Some((new_env_state.head_commit.to_short_ref(), diffs)))
    }

    pub fn prepare(&self, env: &EnvironmentConfig, force_clean: bool) -> Result<()> {
        let repo = Repo::open()?;
        let head_files = if force_clean {
            Some(env.head_filters())
        } else {
            None
        };
        let ignore_list = self.ignore_list();
        repo.checkout_head(head_files, ignore_list.clone())?;
        let head_patterns: Vec<_> = env.head_file_patterns().collect();
        for file_buf in env.propagated_files() {
            let file = file_buf.to_str().unwrap().to_string();
            if !ignore_list.iter().any(|p| p.matches(&file))
                && !head_patterns.iter().any(|p| p.matches(&file))
            {
                std::fs::remove_file(file_buf).expect("Couldn't remove file");
            }
        }
        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = self.db.get_target_propagated_state(&env.name, previous_env) {
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, state) in env_state.files.iter() {
                    if patterns.iter().any(|p| p.matches(&name))
                        && !head_patterns.iter().any(|p| p.matches(&name))
                    {
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
    ) -> Result<(String, Vec<FileDiff>)> {
        eprintln!("Recording current state");
        let repo = Repo::open()?;
        let new_env_state = self.construct_env_state(&repo, &env, true)?;
        let head_commit = new_env_state.head_commit.to_short_ref();
        let diffs = if let Some(last_state) = self.db.get_current_state(&env.name) {
            new_env_state.diff(last_state)
        } else {
            new_env_state
                .files
                .iter()
                .map(|(path, state)| FileDiff {
                    path: path.clone(),
                    current_state: Some(state.clone()),
                    added: true,
                })
                .collect()
        };
        let state_file = self.db.set_current_environment_state(
            env.name.clone(),
            env.propagated_from().cloned(),
            new_env_state,
        )?;
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
        Ok((head_commit, diffs))
    }

    #[allow(clippy::redundant_closure)]
    fn construct_env_state(
        &self,
        repo: &Repo,
        env: &EnvironmentConfig,
        recording: bool,
    ) -> Result<DeployState> {
        let current_commit = repo.head_commit_hash()?;
        let database = Database::open_env(
            &self.path_to_config,
            &env.name,
            env.propagated_from(),
            current_commit.clone(),
            &repo,
        )?;

        let mut best_state = self.construct_state_for_commit(
            &repo,
            current_commit.clone(),
            &env,
            &database,
            recording,
        )?;
        repo.walk_commits_before(current_commit, |commit| {
            if let Some(state) =
                self.get_state_if_equivalent(&env.name, &repo, &best_state, commit, recording)?
            {
                best_state = state;
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        Ok(best_state)
    }

    fn get_state_if_equivalent(
        &self,
        env_name: &str,
        repo: &Repo,
        last_state: &DeployState,
        commit: CommitHash,
        recording: bool,
    ) -> Result<Option<DeployState>> {
        let config = if let Some(config) =
            repo.get_file_content(commit.clone(), Path::new(&self.path_to_config), |bytes| {
                Config::from_reader(bytes)
            })? {
            config
        } else {
            return Ok(None);
        };
        let env = if let Some(env) = config.environments.get(env_name) {
            env
        } else {
            return Ok(None);
        };
        let database = Database::open_env(
            &self.path_to_config,
            &env.name,
            env.propagated_from(),
            commit.clone(),
            &repo,
        )?;
        let new_state =
            self.construct_state_for_commit(&repo, commit, &env, &database, recording)?;
        if last_state.diff(&new_state).is_empty() {
            Ok(Some(new_state))
        } else {
            Ok(None)
        }
    }

    fn construct_state_for_commit(
        &self,
        repo: &Repo,
        commit: CommitHash,
        env: &EnvironmentConfig,
        database: &Database,
        recording: bool,
    ) -> Result<DeployState> {
        let mut new_env_state = DeployState::new(commit.clone());
        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = database.get_target_propagated_state(&env.name, previous_env) {
                new_env_state.propagated_head = Some(env_state.head_commit.clone());
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, prev_state) in env_state.files.iter() {
                    if let Some(last_hash) = prev_state.file_hash.as_ref() {
                        if patterns.iter().any(|p| p.matches(&name)) {
                            let (dirty, file_hash) = if recording {
                                if let Some(file_hash) = hash_file(name) {
                                    (&file_hash != last_hash, Some(file_hash))
                                } else {
                                    (true, None)
                                }
                            } else {
                                (false, Some(last_hash.clone()))
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
        }
        let ignore_list = vec![
            glob::Pattern::new(&self.path_to_config).unwrap(),
            glob::Pattern::new(&format!("{}/*", database.state_dir)).unwrap(),
        ];
        repo.all_files(commit.clone(), |file_hash, path| {
            if env.head_file_patterns().any(|p| p.matches_path(path))
                && !ignore_list.iter().any(|p| p.matches_path(path))
            {
                let (from_commit, message) =
                    repo.find_last_changed_commit(&path, commit.clone())?;
                let state = if recording {
                    if let Some(on_disk_hash) = hash_file(path) {
                        FileState {
                            dirty: file_hash != on_disk_hash,
                            file_hash: Some(on_disk_hash),
                            from_commit,
                            message,
                        }
                    } else {
                        FileState {
                            dirty: true,
                            file_hash: None,
                            from_commit,
                            message,
                        }
                    }
                } else {
                    FileState {
                        dirty: false,
                        file_hash: Some(file_hash),
                        from_commit,
                        message,
                    }
                };
                let file_name = path.to_str().unwrap().to_string();
                new_env_state.files.insert(file_name, state);
            }
            Ok(())
        })?;
        Ok(new_env_state)
    }

    fn ignore_list(&self) -> Vec<glob::Pattern> {
        vec![
            glob::Pattern::new(&self.path_to_config).unwrap(),
            glob::Pattern::new(&format!("{}/*", self.db.state_dir)).unwrap(),
            glob::Pattern::new(".git/*").unwrap(),
            glob::Pattern::new(".gitignore").unwrap(),
        ]
    }
}
