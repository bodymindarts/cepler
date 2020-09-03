use super::{config::EnvironmentConfig, database::*, git::*};
use thiserror::Error;

pub struct Workspace {
    db: Database,
}
impl Workspace {
    pub fn new(path_to_state: String) -> Result<Self, WorkspaceError> {
        Ok(Self {
            db: Database::open(path_to_state)?,
        })
    }
    pub fn prepare(&self, env: &EnvironmentConfig) -> Result<(), WorkspaceError> {
        let repo = Repo::open()?;
        repo.checkout_head()?;
        for file in env.propagated_files() {
            std::fs::remove_file(file).expect("Couldn't remove file");
        }
        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = self.db.get_target_propagated_state(&env.name, previous_env) {
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, state) in env_state.files.iter() {
                    if patterns.iter().any(|p| p.matches(&name)) {
                        repo.checkout_file_from(
                            name,
                            state.from_commit.as_ref().unwrap_or(&env_state.head_commit),
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn record_env(&mut self, env: &EnvironmentConfig) -> Result<(), WorkspaceError> {
        let new_env_state = self.construct_env_state(env)?;
        Ok(self
            .db
            .set_current_environment_state(env.name.clone(), new_env_state)?)
    }

    fn construct_env_state(&self, env: &EnvironmentConfig) -> Result<DeployState, WorkspaceError> {
        let repo = Repo::open()?;
        let head_commit = repo.head_commit_hash()?;
        let mut new_env_state = DeployState::new(head_commit);

        for file in repo.head_files(env.head_filters()) {
            let dirty = repo.is_file_dirty(&file)?;
            let file_name = file.to_str().unwrap().to_string();
            let file_hash = hash_file(file);
            new_env_state.files.insert(
                file_name,
                FileState {
                    file_hash,
                    dirty,
                    from_commit: None,
                },
            );
        }

        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = self.db.get_target_propagated_state(&env.name, previous_env) {
                new_env_state.propagated_head = Some(env_state.head_commit.clone());
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, prev_state) in env_state.files.iter() {
                    if patterns.iter().any(|p| p.matches(&name)) {
                        let file_hash = hash_file(name);
                        let file_state = FileState {
                            dirty: file_hash != prev_state.file_hash,
                            file_hash,
                            from_commit: Some(
                                prev_state
                                    .from_commit
                                    .as_ref()
                                    .map(Clone::clone)
                                    .unwrap_or(env_state.head_commit.clone()),
                            ),
                        };
                        new_env_state.files.insert(name.clone(), file_state);
                    }
                }
            }
        }

        Ok(new_env_state)
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    DbError(#[from] DatabaseError),
    #[error("Error interfacing with git - {0}")]
    GitError(String),
}

impl From<git2::Error> for WorkspaceError {
    fn from(err: git2::Error) -> Self {
        WorkspaceError::GitError(err.message().to_string())
    }
}
