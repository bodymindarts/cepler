use super::config::MATCH_OPTIONS;
use anyhow::*;
use git2::{
    build::CheckoutBuilder, BranchType, Commit, Cred, MergeOptions, Object, ObjectType, Oid,
    PushOptions, RebaseOptions, RemoteCallbacks, Repository, ResetType, Signature, TreeWalkMode,
    TreeWalkResult,
};
use glob::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    fmt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileHash(String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitHash(String);
impl fmt::Display for CommitHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.chars().take(7).collect::<String>())
    }
}
impl CommitHash {
    pub fn inner(self) -> String {
        self.0
    }

    pub fn to_short_ref(&self) -> String {
        self.0.chars().take(7).collect()
    }
}

pub fn hash_file<P: AsRef<Path>>(file: P) -> Option<FileHash> {
    let path = file.as_ref();
    if path.is_file() {
        Some(FileHash(
            Oid::hash_file(ObjectType::Blob, path)
                .expect("Couldn't hash object")
                .to_string(),
        ))
    } else {
        None
    }
}

pub struct GitConfig {
    pub url: String,
    pub branch: String,
    pub gates_branch: Option<String>,
    pub private_key: String,
    pub dir: String,
}

pub struct Repo {
    inner: Repository,
    gate: Option<Oid>,
}

impl Repo {
    pub fn clone(
        GitConfig {
            url,
            branch,
            private_key,
            dir,
            ..
        }: GitConfig,
    ) -> Result<Self> {
        let callbacks = remote_callbacks(private_key);
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(&branch);
        let inner = builder.clone(&url, Path::new(&dir))?;
        Ok(Self { inner, gate: None })
    }

    pub fn pull(
        &self,
        GitConfig {
            branch,
            gates_branch,
            private_key,
            ..
        }: GitConfig,
    ) -> Result<()> {
        let callbacks = remote_callbacks(private_key);
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut remote = self.inner.find_remote("origin")?;
        let mut branches = vec![branch.clone()];
        if let Some(gates) = gates_branch {
            branches.push(gates);
        }
        remote.fetch(&branches, Some(&mut fo), None)?;
        let suffix = format!("/{}", branch);
        let remote_head = remote
            .list()?
            .iter()
            .find(|head| head.name().ends_with(&suffix))
            .context("Cannot find head")?;
        let object = self
            .inner
            .find_object(remote_head.oid(), Some(ObjectType::Commit))?;
        self.inner.reset(&object, ResetType::Hard, None)?;
        Ok(())
    }

    pub fn push(
        &self,
        GitConfig {
            branch,
            private_key,
            ..
        }: GitConfig,
    ) -> Result<()> {
        let callbacks = remote_callbacks(private_key.clone());
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut remote = self.inner.find_remote("origin")?;
        remote.fetch(&[branch.clone()], Some(&mut fo), None)?;

        let head_commit = self
            .inner
            .reference_to_annotated_commit(&self.inner.head()?)?;
        let branch_ref = self
            .inner
            .branch_from_annotated_commit(&branch, &head_commit, true)?;
        let head_commit = self.inner.reference_to_annotated_commit(branch_ref.get())?;

        let remote_ref = self
            .inner
            .resolve_reference_from_short_name(&format!("origin/{}", branch))?;
        let remote_commit = self.inner.reference_to_annotated_commit(&remote_ref)?;

        let mut rebase_options = RebaseOptions::new();
        let mut merge_options = MergeOptions::new();
        merge_options.fail_on_conflict(true);
        rebase_options.merge_options(merge_options);
        let mut rebase = self.inner.rebase(
            Some(&head_commit),
            Some(&remote_commit),
            None,
            Some(&mut rebase_options),
        )?;
        let sig = Signature::now("Cepler", "bot@cepler.dev")?;
        while let Some(_) = rebase.next() {
            rebase.commit(None, &sig, None)?;
        }
        rebase.finish(None)?;

        let mut push_options = PushOptions::new();
        push_options.remote_callbacks(remote_callbacks(private_key));
        remote.push(
            &[format!(
                "{}:{}",
                head_commit.refname().unwrap(),
                head_commit.refname().unwrap(),
            )],
            Some(&mut push_options),
        )?;
        Ok(())
    }

    pub fn open(gate: Option<String>) -> Result<Self> {
        let inner = Repository::open_from_env()?;
        let gate = if let Some(gate) = gate {
            let commit = Oid::from_str(&gate).context("Gate is not a valid commit hash")?;
            inner
                .find_commit(commit)
                .context("Gate commit doesn't exist")?;
            Some(commit)
        } else {
            None
        };
        Ok(Self { inner, gate })
    }

    pub fn commit_state_file(&self, file_name: String) -> Result<()> {
        let path = Path::new(&file_name);
        let mut index = self.inner.index()?;
        index.add_path(path)?;
        let oid = index.write_tree()?;
        let tree = self.inner.find_tree(oid)?;
        let sig = Signature::now("Cepler", "bot@cepler.io")?;

        let head_commit = self.inner.head().unwrap().peel_to_commit().unwrap();
        self.inner.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &format!(
                "[cepler] Updated {} state",
                path.file_stem().unwrap().to_str().unwrap()
            ),
            &tree,
            &[&head_commit],
        )?;
        let mut checkout = CheckoutBuilder::new();
        checkout.path(path);
        self.inner.checkout_index(None, Some(&mut checkout))?;
        Ok(())
    }

    fn gate_files_matching<'a>(
        &self,
        globs: &'a [Pattern],
        ignore_files: &'a [Pattern],
    ) -> impl Iterator<Item = PathBuf> + 'a {
        let ignore = move |file: &Path| {
            ignore_files
                .iter()
                .any(|p| p.matches_path_with(file, MATCH_OPTIONS))
        };
        let includes = move |file: &Path| {
            globs
                .iter()
                .any(|p| p.matches_path_with(file, MATCH_OPTIONS))
        };
        let mut paths = Vec::new();
        self.all_files(self.gate_commit_hash(), |_, path| {
            if !ignore(path) && includes(path) {
                paths.push(path.to_path_buf())
            }
            Ok(())
        })
        .expect("Couldn't list gate files");
        paths.into_iter()
    }

    pub fn all_files<F>(&self, commit: CommitHash, mut f: F) -> Result<()>
    where
        F: FnMut(FileHash, &Path) -> Result<()>,
    {
        let commit = Oid::from_str(&commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        let tree = commit.tree().context("Couldn't resolve tree")?;
        let mut ret = Ok(());
        tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
            let path_name = format!("{}{}", dir, entry.name().expect("Entry has no name"));
            let path = Path::new(&path_name);
            if let Some(ObjectType::Blob) = entry.kind() {
                if let Err(e) = f(FileHash(entry.id().to_string()), path) {
                    ret = Err(e);
                    return TreeWalkResult::Abort;
                }
            }
            TreeWalkResult::Ok
        })?;
        ret
    }

    fn is_trackable_file(&self, file: &Path) -> bool {
        if self.inner.status_file(file).is_err() {
            return false;
        }
        !self
            .inner
            .status_should_ignore(file)
            .expect("Cannot check ignore status")
    }

    pub fn gate_commit_hash(&self) -> CommitHash {
        CommitHash(self.gate_oid().to_string())
    }

    pub fn head_commit_hash(&self) -> Result<CommitHash> {
        Ok(CommitHash(
            self.inner
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .id()
                .to_string(),
        ))
    }

    pub fn checkout_file_from(&self, path: &str, commit: &CommitHash) -> Result<()> {
        let object = self.inner.find_object(
            Oid::from_str(&commit.0).expect("Couldn't parse Oid"),
            Some(ObjectType::Commit),
        )?;
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.path(path);
        checkout.update_index(false);
        self.inner.checkout_tree(&object, Some(&mut checkout))?;

        Ok(())
    }

    pub fn checkout_gate(
        &self,
        globs: &[Pattern],
        ignore_files: &[Pattern],
        clean: bool,
    ) -> Result<()> {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.update_index(false);
        let mut path_added = false;
        for path in self.gate_files_matching(globs, ignore_files) {
            path_added = true;
            checkout.path(path);
        }

        for path in glob("**/*").expect("List all files") {
            let path = path.expect("Get file");
            if self.is_trackable_file(&path) {
                let path = path.as_path();
                let check = |p: &glob::Pattern| {
                    p.matches_path_with(
                        path,
                        glob::MatchOptions {
                            case_sensitive: true,
                            require_literal_separator: true,
                            require_literal_leading_dot: true,
                        },
                    )
                };
                if !ignore_files.iter().any(|p| check(p)) && path.is_file() {
                    if clean || globs.iter().any(|p| check(p)) {
                        std::fs::remove_file(path).expect("Couldn't remove file");
                    }
                }
            }
        }
        if path_added {
            self.inner
                .checkout_tree(&self.gate_object(), Some(&mut checkout))
                .expect("Couldn't checkout");
        }
        Ok(())
    }

    pub fn checkout_head(&self) -> Result<()> {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.update_index(false);
        self.inner.checkout_head(Some(&mut checkout))?;
        Ok(())
    }

    pub fn walk_commits_before<F>(&self, commit: CommitHash, mut cb: F) -> Result<()>
    where
        F: FnMut(CommitHash) -> Result<bool>,
    {
        let commit = Oid::from_str(&commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        let mut set = HashSet::new();
        let mut queue = VecDeque::new();
        set.insert(commit.id());
        for parent in commit.parents() {
            if set.insert(parent.id()) {
                queue.push_back(parent);
            }
        }
        loop {
            if queue.is_empty() {
                break;
            }
            let commit = queue.pop_front().unwrap();
            if !cb(CommitHash(commit.id().to_string()))? {
                break;
            }
            for parent in commit.parents() {
                if set.insert(parent.id()) {
                    queue.push_back(parent);
                }
            }
        }
        Ok(())
    }

    pub fn find_last_changed_commit(
        &self,
        file: &Path,
        from_commit: CommitHash,
    ) -> Result<(CommitHash, String)> {
        let commit = Oid::from_str(&from_commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        let target = commit
            .tree()
            .context("Couldn't resolve tree")?
            .get_path(file)
            .context("Trying to record uncommitted file")?;
        let mut set = HashSet::new();
        let mut queue = VecDeque::new();
        set.insert(commit.id());
        queue.push_back(commit);

        loop {
            let commit = queue.pop_front().unwrap();
            let mut go = false;
            for parent in commit.parents() {
                if let Ok(tree) = parent.tree().expect("Couldn't get tree").get_path(file) {
                    let eq = tree.id() == target.id();
                    if eq && set.insert(parent.id()) {
                        queue.push_back(parent);
                    }
                    go = go || eq;
                }
            }
            if !go || queue.is_empty() {
                return Ok((
                    CommitHash(commit.id().to_string()),
                    commit.summary().expect("Couldn't get summary").to_string(),
                ));
            }
        }
    }

    pub fn get_file_content<F, T>(&self, commit: CommitHash, file: &Path, f: F) -> Result<Option<T>>
    where
        F: Fn(&[u8]) -> Result<T>,
    {
        let commit = Oid::from_str(&commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        self.get_file_from_commit(commit, file, f)
    }

    fn get_file_from_commit<F, T>(&self, commit: Commit, file: &Path, f: F) -> Result<Option<T>>
    where
        F: Fn(&[u8]) -> Result<T>,
    {
        let tree = commit.tree().context("Couldn't resolve tree")?;
        let target = if let Ok(target) = tree.get_path(file) {
            target
        } else {
            return Ok(None);
        };
        let object = target
            .to_object(&self.inner)
            .context("Couldn't create object")?;
        let blob = object.peel_to_blob().context("Couldn't peel to blob")?;
        Ok(Some(f(blob.content())?))
    }

    fn gate_commit(&self) -> Commit<'_> {
        if let Some(gate) = self.gate {
            self.inner.find_commit(gate).unwrap()
        } else {
            self.inner.head().unwrap().peel_to_commit().unwrap()
        }
    }

    fn gate_oid(&self) -> Oid {
        self.gate_commit().id()
    }

    fn gate_object(&self) -> Object {
        self.inner
            .find_object(self.gate_oid(), Some(ObjectType::Commit))
            .unwrap()
    }

    pub fn get_file_from_branch<F, T>(&self, name: &str, file: &Path, f: F) -> Result<Option<T>>
    where
        F: Fn(&[u8]) -> Result<T>,
    {
        let branch = if let Ok(branch) = self.inner.find_branch(name, BranchType::Local) {
            branch
        } else {
            self.inner
                .find_branch(&format!("origin/{}", name), BranchType::Remote)
                .context("Couldn't find branch")?
        };

        self.get_file_from_commit(branch.into_reference().peel_to_commit()?, file, f)
    }
}

fn remote_callbacks(key: String) -> RemoteCallbacks<'static> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, username_from_url, _allowed_types| {
        Cred::ssh_key_from_memory(username_from_url.unwrap(), None, &key, None)
    });
    callbacks
}
