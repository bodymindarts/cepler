use super::{config::*, git::*};
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
        let repo = Repo::open()?;
        let commit_hash = repo.head_commit_hash()?;
        let mut env_state = EnvironmentState::new(commit_hash);

        let mut any_dirty = false;
        for file in env.all_files() {
            let dirty = repo.is_file_dirty(&file)?;
            any_dirty = any_dirty || dirty;
            let file_name = file.to_str().unwrap().to_string();
            let file_hash = hash_file(file);
            env_state
                .files
                .insert(file_name, FileState { file_hash, dirty });
        }
        env_state.any_dirty = any_dirty;
        self.state.environments.insert(env.name.clone(), env_state);
        self.persist()
    }

    fn persist(&self) -> Result<(), DatabaseError> {
        use std::io::Write;
        let mut file = File::create(&self.state_file)?;
        file.write_all(&serde_yaml::to_vec(&self.state)?)?;
        Ok(())
    }

    pub fn environment(&self, env: &String) -> Option<&EnvironmentState> {
        self.state.environments.get(env)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DbState {
    environments: HashMap<String, EnvironmentState>,
}

impl DbState {
    fn from_reader(reader: impl Read) -> Result<Self, DatabaseError> {
        let state = serde_yaml::from_reader(reader)?;
        Ok(state)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnvironmentState {
    pub commit_hash: CommitHash,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    any_dirty: bool,
    #[serde(default)]
    pub files: HashMap<String, FileState>,
}

impl EnvironmentState {
    fn new(commit_hash: CommitHash) -> Self {
        Self {
            commit_hash,
            any_dirty: false,
            files: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileState {
    file_hash: FileHash,
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
