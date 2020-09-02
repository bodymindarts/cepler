use super::config::*;
use git2::{ObjectType, Oid, Repository, Status};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
use thiserror::Error;

pub struct Database {
    state: DbState,
    state_file: String,
}

impl Database {
    pub fn open(file: String) -> Result<Self, DatabaseError> {
        let path = Path::new(&file);
        let state = if path.exists() {
            let file = File::open(path)?;
            let reader = BufReader::new(file);

            DbState::from_reader(reader)?
        } else {
            DbState::default()
        };
        Ok(Self {
            state,
            state_file: file,
        })
    }

    pub fn record_env(&mut self, env: &EnvironmentConfig) -> Result<(), DatabaseError> {
        let mut env_state = EnvironmentState::default();
        let repo = Repository::open_from_env()?;
        for file in env.files() {
            let dirty = !repo.status_file(file.as_path())?.is_empty();
            let file_name = file.to_str().unwrap().to_string();
            let file_hash = Oid::hash_file(ObjectType::Blob, file)
                .expect("Couldn't hash object")
                .to_string();
            env_state
                .files
                .insert(file_name, FileState { file_hash, dirty });
        }
        let commit_hash = repo.head()?.peel_to_commit()?.id().to_string();
        env_state.commit_hash = commit_hash;
        self.state.environments.insert(env.name.clone(), env_state);
        self.persist()
    }

    fn persist(&self) -> Result<(), DatabaseError> {
        use std::io::Write;
        let mut file = File::create(&self.state_file)?;
        file.write_all(&serde_yaml::to_vec(&self.state)?)?;
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DbState {
    environments: HashMap<String, EnvironmentState>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
struct EnvironmentState {
    commit_hash: String,
    #[serde(default)]
    files: HashMap<String, FileState>,
}
impl DbState {
    fn from_reader(reader: impl Read) -> Result<Self, DatabaseError> {
        let state = serde_yaml::from_reader(reader)?;
        Ok(state)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileState {
    file_hash: String,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    dirty: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Could not read the database file: '{0}'")]
    UnknownFormat(#[from] serde_yaml::Error),
    #[error("Could not open the database file: '{0}'")]
    CouldNotOpen(#[from] std::io::Error),
    #[error("Error interfacing with git: '{0}'")]
    GitError(String),
}

impl From<git2::Error> for DatabaseError {
    fn from(err: git2::Error) -> Self {
        DatabaseError::GitError(err.message().to_string())
    }
}
