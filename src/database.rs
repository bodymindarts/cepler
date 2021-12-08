use super::{config::*, repo::*};
use anyhow::*;
use glob::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fmt,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

pub struct Database {
    state: DbState,
    ignore_queue: bool,
    pub state_dir: String,
}

const STATE_DIR: &str = ".cepler";

impl Database {
    pub fn state_dir_from_config(scope: &str, path_to_config: &str) -> String {
        let path = Path::new(path_to_config);
        format!(
            "{}/{}",
            match path.parent() {
                Some(parent) if parent == Path::new("") => STATE_DIR.to_string(),
                None => STATE_DIR.to_string(),
                Some(parent) => format!("{}/{}", parent.to_str().unwrap(), STATE_DIR),
            },
            scope
        )
    }

    pub fn open(scope: &str, path_to_config: &str, ignore_queue: bool) -> Result<Self> {
        let mut state = DbState::default();
        let dir = Self::state_dir_from_config(scope, path_to_config);
        if Path::new(&dir).is_dir() {
            for path in glob(&format!("{}/*.state", dir))? {
                let path = path?;
                if let Some(name) = path.as_path().file_stem() {
                    let file = File::open(&path)?;
                    let reader = BufReader::new(file);
                    state.environments.insert(
                        name.to_str().expect("Convert name").to_string(),
                        EnvironmentState::from_reader(reader)?,
                    );
                }
            }
        }

        Ok(Self {
            state,
            state_dir: dir,
            ignore_queue,
        })
    }

    pub fn open_env_from_commit(
        &self,
        path_to_config: &str,
        ignore_queue: bool,
        scope: &str,
        env_config: &EnvironmentConfig,
        commit: CommitHash,
        repo: &Repo,
    ) -> Result<Self> {
        let dir = Self::state_dir_from_config(scope, path_to_config);
        let mut state = DbState::default();
        if let Some(env_state) = self.state.environments.get(&env_config.name) {
            state
                .environments
                .insert(env_config.name.to_string(), env_state.clone());
        }
        if let Some(last_env) = env_config.propagated_from() {
            let env_file = format!("{}/{}.state", dir, last_env);
            let env_path = Path::new(&env_file);
            if let Some(env_state) = repo.get_file_content(commit, env_path, |bytes| {
                EnvironmentState::from_reader(bytes)
            })? {
                state.environments.insert(last_env.to_string(), env_state);
            }
        }
        Ok(Self {
            state,
            state_dir: dir,
            ignore_queue,
        })
    }

    pub fn set_current_environment_state(
        &mut self,
        name: String,
        propagated_from: Option<String>,
        mut env: DeployState,
    ) -> Result<String> {
        let any_dirty = env.files.values().any(|f| f.dirty);
        env.any_dirty = any_dirty;
        let ret = format!("{}/{}.state", self.state_dir, &name);
        if let Some(state) = self.state.environments.get_mut(&name) {
            std::mem::swap(&mut state.current, &mut env);
            state.propagation_queue.push_front(env);
            state.propagated_from = propagated_from;
        } else {
            self.state.environments.insert(
                name.clone(),
                EnvironmentState {
                    current: env,
                    propagated_from,
                    propagation_queue: VecDeque::new(),
                },
            );
        }
        self.state.prune_propagation_queue(name);
        self.persist()?;
        Ok(ret)
    }

    pub fn get_target_propagated_state(
        &self,
        env: &str,
        env_ignore_queue: bool,
        propagated_from: &str,
        patterns: &[glob::Pattern],
    ) -> Option<&DeployState> {
        let match_options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        match (
            self.state.environments.get(env),
            self.state.environments.get(propagated_from),
        ) {
            (Some(env), Some(from)) => {
                if let Some(from_head) = env.current.propagated_head.as_ref() {
                    if self.ignore_queue
                        || env_ignore_queue
                        || from_head == &from.current.head_commit
                        || from.propagation_queue.is_empty()
                    {
                        Some(&from.current)
                    } else {
                        let mut ret = &from.current;
                        for state in from.propagation_queue.iter() {
                            if &state.head_commit == from_head {
                                break;
                            }
                            for (ident, file_state) in state.files.iter() {
                                let file_name = ident.name();
                                if patterns
                                    .iter()
                                    .any(|p| p.matches_with(&file_name, match_options))
                                {
                                    if let Some((_, existing_state)) = env
                                        .current
                                        .files
                                        .iter()
                                        .find(|(ident, _)| ident.name() == file_name)
                                    {
                                        if existing_state.file_hash != file_state.file_hash {
                                            ret = state;
                                            break;
                                        }
                                    } else {
                                        ret = state;
                                        break;
                                    }
                                }
                            }
                        }
                        Some(ret)
                    }
                } else {
                    Some(&from.current)
                }
            }
            (None, Some(state)) => Some(&state.current),
            _ => None,
        }
    }

    pub fn get_current_state(&self, env: &str) -> Option<&DeployState> {
        self.state.environments.get(env).map(|env| &env.current)
    }

    fn persist(&self) -> Result<()> {
        use std::fs;
        use std::io::Write;
        let _ = fs::remove_dir_all(&self.state_dir);
        fs::create_dir_all(&self.state_dir)?;
        for (name, env) in self.state.environments.iter() {
            let mut file = File::create(&format!("{}/{}.state", self.state_dir, name))?;
            let mut bytes = serde_yaml::to_vec(&env)?;
            bytes.extend("\n".as_bytes());
            file.write_all(&bytes)?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DbState {
    environments: BTreeMap<String, EnvironmentState>,
}

impl DbState {
    fn prune_propagation_queue(&mut self, name: String) {
        let mut keep_states = 0;
        let to_prune = self.environments.get(&name).unwrap();
        for commit_hash in self.environments.iter().filter_map(|(env_name, state)| {
            if env_name == &name
                || state.propagated_from.is_none()
                || state.propagated_from.as_ref().unwrap() != &name
            {
                None
            } else {
                state.current.propagated_head.as_ref()
            }
        }) {
            if commit_hash == &to_prune.current.head_commit {
                continue;
            }
            for (idx, old_hash) in to_prune
                .propagation_queue
                .iter()
                .map(|state| &state.head_commit)
                .enumerate()
                .skip(keep_states)
            {
                if old_hash == commit_hash {
                    break;
                }
                keep_states = keep_states.max(idx + 1);
            }
        }
        let to_prune = self.environments.get_mut(&name).unwrap();
        to_prune.propagation_queue.drain(keep_states..);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentState {
    current: DeployState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagated_from: Option<String>,
    #[serde(skip_serializing_if = "VecDeque::is_empty")]
    #[serde(default)]
    propagation_queue: VecDeque<DeployState>,
}

impl EnvironmentState {
    fn from_reader(reader: impl Read) -> Result<Self> {
        let state = serde_yaml::from_reader(reader)?;
        Ok(state)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployState {
    pub head_commit: CommitHash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagated_head: Option<CommitHash>,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    any_dirty: bool,
    #[serde(default)]
    pub files: BTreeMap<FileIdent, FileState>,
}

#[derive(Debug, Clone, Hash, PartialOrd, PartialEq, Eq, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileIdent(String);
impl FileIdent {
    pub fn new(name: String, from: Option<&str>) -> Self {
        Self(format!(
            "{{{}}}/{}",
            from.as_ref().unwrap_or(&"latest"),
            name
        ))
    }

    pub fn name(&self) -> String {
        self.0.chars().skip_while(|c| c != &'}').skip(2).collect()
    }

    pub fn inner(self) -> String {
        self.0
    }
}

impl DeployState {
    pub fn new(head_commit: CommitHash) -> Self {
        Self {
            head_commit,
            propagated_head: None,
            any_dirty: false,
            files: BTreeMap::new(),
        }
    }

    pub fn diff(&self, other: &DeployState) -> Vec<FileDiff> {
        let mut removed_files: HashSet<&FileIdent> = other.files.keys().collect();
        let mut diffs: Vec<_> = self
            .files
            .iter()
            .filter_map(|(ident, state)| {
                removed_files.remove(&ident);
                if let Some(last_state) = other.files.get(ident) {
                    if state.file_hash.is_none() && last_state.file_hash.is_none() {
                        None
                    } else if state.dirty
                        || last_state.dirty
                        || state.file_hash != last_state.file_hash
                    {
                        Some(FileDiff {
                            ident: ident.clone(),
                            current_state: if state.file_hash.is_some() {
                                Some(state.clone())
                            } else {
                                None
                            },
                            added: last_state.file_hash.is_none(),
                        })
                    } else {
                        None
                    }
                } else {
                    Some(FileDiff {
                        ident: ident.clone(),
                        current_state: if state.file_hash.is_some() {
                            Some(state.clone())
                        } else {
                            None
                        },
                        added: true,
                    })
                }
            })
            .collect();
        diffs.extend(removed_files.iter().map(|ident| FileDiff {
            ident: FileIdent::clone(ident),
            current_state: None,
            added: false,
        }));
        diffs
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub ident: FileIdent,
    pub current_state: Option<FileState>,
    pub added: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileState {
    pub file_hash: Option<FileHash>,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    pub dirty: bool,
    pub from_commit: CommitHash,
    pub message: String,
}

impl fmt::Display for FileState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] - {}",
            self.from_commit.to_short_ref(),
            self.message
        )
    }
}

fn is_false(b: &bool) -> bool {
    !b
}
