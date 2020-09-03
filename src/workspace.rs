use super::{config::EnvironmentConfig, database::*, git::*};
use thiserror::Error;

pub struct Workspace {
    db: Database,
}
impl Workspace {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    pub fn prepare(&self, env: &EnvironmentConfig) -> Result<(), WorkspaceError> {
        if let Some(previous_env) = env.propagated_from() {
            let repo = Repo::open()?;
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
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("Error interfacing with git: '{0}'")]
    GitError(String),
}

impl From<git2::Error> for WorkspaceError {
    fn from(err: git2::Error) -> Self {
        WorkspaceError::GitError(err.message().to_string())
    }
}
