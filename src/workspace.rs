use super::{config::*, database::*, repo::*};
use anyhow::*;
use std::path::Path;

pub struct Workspace {
    path_to_config: String,
    scope: String,
    ignore_queue: bool,
    db: Database,
}

pub struct StateId {
    pub head_commit: String,
    pub version: u32,
}

impl Workspace {
    pub fn new(scope: &str, path_to_config: String, ignore_queue: bool) -> Result<Self> {
        Ok(Self {
            db: Database::open(scope, &path_to_config, ignore_queue)?,
            scope: scope.to_string(),
            path_to_config,
            ignore_queue,
        })
    }

    pub fn ls(&self, env: &EnvironmentConfig, gate: Option<String>) -> Result<Vec<String>> {
        let repo = Repo::open(gate)?;
        let new_env_state = self.construct_env_state(&repo, env, false)?;
        Ok(new_env_state
            .files
            .into_iter()
            .map(|(k, _)| k.name())
            .collect())
    }

    pub fn check(
        &self,
        env: &EnvironmentConfig,
        gate: Option<String>,
    ) -> Result<Option<(StateId, Vec<FileDiff>)>> {
        let repo = Repo::open(gate)?;
        if let Some(previous_env) = env.propagated_from() {
            self.db.get_current_state(previous_env).context(format!(
                "Previous environment '{}' not deployed yet",
                previous_env
            ))?;
        }
        let new_env_state = self.construct_env_state(&repo, env, false)?;
        let (version, diffs) = if let Some((version, last)) = self.db.get_current_state(&env.name) {
            let diffs = new_env_state.diff(last);
            if diffs.is_empty() {
                return Ok(None);
            }
            (version + 1, diffs)
        } else {
            (
                1,
                new_env_state
                    .files
                    .iter()
                    .map(|(ident, state)| FileDiff {
                        ident: ident.clone(),
                        current_state: Some(state.clone()),
                        added: true,
                    })
                    .collect(),
            )
        };
        for diff in diffs.iter() {
            let name = diff.ident.name();
            if diff.added {
                eprintln!("File {} was added", name)
            } else if diff.current_state.is_some() {
                eprintln!("File {} changed", name)
            } else {
                eprintln!("File {} was removed", name)
            }
        }
        Ok(Some((
            StateId {
                version,
                head_commit: new_env_state.head_commit.inner(),
            },
            diffs,
        )))
    }

    pub fn reproduce(&self, env: &EnvironmentConfig, force_clean: bool) -> Result<StateId> {
        let repo = Repo::open(None)?;
        if let Some((version, last_state)) = self.db.get_current_state(&env.name) {
            if force_clean {
                repo.checkout_gate(&[], &self.ignore_list(), true)?;
            }
            for (ident, state) in last_state.files.iter() {
                repo.checkout_file_from(&ident.name(), &state.from_commit)?;
            }
            Ok(StateId {
                version,
                head_commit: last_state.head_commit.clone().inner(),
            })
        } else {
            Err(anyhow!("No state recorded for {}", env.name))
        }
    }

    pub fn prepare(
        &self,
        env: &EnvironmentConfig,
        gate: Option<String>,
        force_clean: bool,
    ) -> Result<()> {
        let repo = Repo::open(gate)?;
        let ignore_list = self.ignore_list();
        let head_patterns: Vec<_> = env.head_file_patterns().collect();
        repo.checkout_gate(&head_patterns, &ignore_list, force_clean)?;
        for file_buf in env.propagated_files() {
            let file = file_buf.as_path();
            if file.is_file()
                && !ignore_list
                    .iter()
                    .any(|p| p.matches_path_with(file, MATCH_OPTIONS))
                && !head_patterns
                    .iter()
                    .any(|p| p.matches_path_with(file, MATCH_OPTIONS))
            {
                std::fs::remove_file(file_buf).expect("Couldn't remove file");
            }
        }
        if let Some(previous_env) = env.propagated_from() {
            let patterns: Vec<_> = env.propagated_file_patterns().collect();
            if let Some(env_state) = self.db.get_target_propagated_state(
                &env.name,
                env.ignore_queue,
                previous_env,
                &patterns,
            ) {
                for (ident, state) in env_state.files.iter() {
                    let name = ident.name();
                    if patterns
                        .iter()
                        .any(|p| p.matches_with(&name, MATCH_OPTIONS))
                        && !head_patterns
                            .iter()
                            .any(|p| p.matches_with(&name, MATCH_OPTIONS))
                    {
                        repo.checkout_file_from(&name, &state.from_commit)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn record_env(
        &mut self,
        env: &EnvironmentConfig,
        gate: Option<String>,
        commit: bool,
        reset: bool,
        git_config: Option<GitConfig>,
    ) -> Result<(StateId, Vec<FileDiff>)> {
        eprintln!("Recording current state");
        let repo = Repo::open(gate)?;
        let new_env_state = self.construct_env_state(&repo, env, true)?;
        let head_commit = new_env_state.head_commit.clone().inner();
        let diffs = if let Some((_, last_state)) = self.db.get_current_state(&env.name) {
            new_env_state.diff(last_state)
        } else {
            new_env_state
                .files
                .iter()
                .map(|(ident, state)| FileDiff {
                    ident: ident.clone(),
                    current_state: Some(state.clone()),
                    added: true,
                })
                .collect()
        };
        let (version, state_file) = self.db.set_current_environment_state(
            env.name.clone(),
            env.propagated_from().cloned(),
            new_env_state,
        )?;
        if commit {
            eprintln!("Adding commit to repository to persist state");
            repo.commit_state_file(&self.scope, state_file)?;
        }
        if reset {
            eprintln!("Reseting head to have a clean workspace");
            repo.checkout_head()?;
        }
        if let Some(config) = git_config {
            eprintln!("Pushing to remote");
            repo.push(config)?;
        }
        Ok((
            StateId {
                head_commit,
                version,
            },
            diffs,
        ))
    }

    #[allow(clippy::redundant_closure)]
    fn construct_env_state(
        &self,
        repo: &Repo,
        env: &EnvironmentConfig,
        recording: bool,
    ) -> Result<DeployState> {
        let current_commit = repo.gate_commit_hash();
        let database = self.db.open_env_from_commit(
            &self.path_to_config,
            self.ignore_queue,
            &self.scope,
            env,
            current_commit.clone(),
            repo,
        )?;

        let mut best_state = self.construct_state_for_commit(
            repo,
            current_commit.clone(),
            env,
            &database,
            recording,
        )?;
        repo.walk_commits_before(current_commit, |commit| {
            if let Some(state) =
                self.get_state_if_equivalent(&env.name, repo, &best_state, commit, recording)?
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
        let database = self.db.open_env_from_commit(
            &self.path_to_config,
            self.ignore_queue,
            &config.scope,
            env,
            commit.clone(),
            repo,
        )?;
        let new_state = self.construct_state_for_commit(repo, commit, env, &database, recording)?;
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
            let patterns: Vec<_> = env.propagated_file_patterns().collect();
            if let Some(env_state) = database.get_target_propagated_state(
                &env.name,
                env.ignore_queue,
                previous_env,
                &patterns,
            ) {
                new_env_state.propagated_head = Some(env_state.head_commit.clone());
                for (ident, prev_state) in env_state.files.iter() {
                    let name = ident.name();
                    if let Some(last_hash) = prev_state.file_hash.as_ref() {
                        if patterns
                            .iter()
                            .any(|p| p.matches_with(&name, MATCH_OPTIONS))
                        {
                            let (dirty, file_hash) = if recording {
                                if let Some(file_hash) = hash_file(&name) {
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
                            new_env_state.files.insert(
                                FileIdent::new(name.clone(), Some(previous_env)),
                                file_state,
                            );
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
            if env
                .head_file_patterns()
                .any(|p| p.matches_path_with(path, MATCH_OPTIONS))
                && !ignore_list
                    .iter()
                    .any(|p| p.matches_path_with(path, MATCH_OPTIONS))
            {
                let (from_commit, message) = repo.find_last_changed_commit(path, commit.clone())?;
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
                new_env_state
                    .files
                    .insert(FileIdent::new(file_name, None), state);
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
