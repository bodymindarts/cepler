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
            if let Some(env_state) = self.db.environment(previous_env) {
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                let mut files_to_checkout = Vec::new();
                for name in env_state.files.keys() {
                    if patterns.iter().any(|p| p.matches(&name)) {
                        files_to_checkout.push(name)
                    }
                }
                repo.checkout_file_from(files_to_checkout, &env_state.head_commit)?;
            }
        }
        Ok(())
    }

    pub fn record_env(&mut self, env: &EnvironmentConfig) -> Result<(), WorkspaceError> {
        let repo = Repo::open()?;
        let head_commit = repo.head_commit_hash()?;
        let mut new_env_state = EnvironmentState::new(head_commit);

        for file in env.head_files() {
            let dirty = repo.is_file_dirty(&file)?;
            let file_name = file.to_str().unwrap().to_string();
            let file_hash = hash_file(file);
            new_env_state
                .files
                .insert(file_name, FileState { file_hash, dirty });
        }

        if let Some(previous_env) = env.propagated_from() {
            if let Some(env_state) = self.db.environment(previous_env) {
                let patterns: Vec<_> = env.propagated_file_patterns().collect();
                for (name, prev_state) in env_state.files.iter() {
                    if patterns.iter().any(|p| p.matches(&name)) {
                        let file_hash = hash_file(name);
                        let file_state = FileState {
                            dirty: file_hash != prev_state.file_hash,
                            file_hash,
                        };
                        new_env_state.files.insert(name.clone(), file_state);
                    }
                }
            }
        }

        Ok(self
            .db
            .set_environment_state(env.name.clone(), new_env_state)?)
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    DbError(#[from] DatabaseError),
    #[error("Error interfacing with git: '{0}'")]
    GitError(String),
}

impl From<git2::Error> for WorkspaceError {
    fn from(err: git2::Error) -> Self {
        WorkspaceError::GitError(err.message().to_string())
    }
}
