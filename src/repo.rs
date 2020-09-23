use anyhow::*;
use git2::{
    build::CheckoutBuilder, Commit, Cred, MergeOptions, ObjectType, Oid, PushOptions,
    RebaseOptions, RemoteCallbacks, Repository, ResetType, Signature,
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
    pub fn to_short_ref(&self) -> String {
        self.0.chars().take(7).collect()
    }
}

pub fn hash_file<P: AsRef<Path>>(file: P) -> FileHash {
    FileHash(
        Oid::hash_file(ObjectType::Blob, file)
            .expect("Couldn't hash object")
            .to_string(),
    )
}

pub struct GitConfig {
    pub url: String,
    pub branch: String,
    pub private_key: String,
    pub dir: String,
}
pub struct Repo {
    inner: Repository,
}

impl Repo {
    pub fn clone(
        GitConfig {
            url,
            branch,
            private_key,
            dir,
        }: GitConfig,
    ) -> Result<Self> {
        let callbacks = remote_callbacks(private_key)?;
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(&branch);
        let inner = builder.clone(&url, Path::new(&dir))?;
        Ok(Self { inner })
    }

    pub fn pull(
        &self,
        GitConfig {
            branch,
            private_key,
            ..
        }: GitConfig,
    ) -> Result<()> {
        let callbacks = remote_callbacks(private_key)?;
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut remote = self.inner.find_remote("origin")?;
        remote.fetch(&[branch.clone()], Some(&mut fo), None)?;
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
        let callbacks = remote_callbacks(private_key.clone())?;
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut remote = self.inner.find_remote("origin")?;
        remote.fetch(&[branch.clone()], Some(&mut fo), None)?;

        let head_commit = self
            .inner
            .reference_to_annotated_commit(&self.inner.head()?)?;
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
        push_options.remote_callbacks(remote_callbacks(private_key)?);
        remote.push(
            &[format!(
                "{}:{}",
                head_commit.refname().unwrap(),
                head_commit.refname().unwrap()
            )],
            Some(&mut push_options),
        )?;
        Ok(())
    }

    pub fn open() -> Result<Self> {
        Ok(Self {
            inner: Repository::open_from_env()?,
        })
    }

    pub fn commit_state_file(&self, file_name: String) -> Result<()> {
        let path = Path::new(&file_name);
        let mut index = self.inner.index()?;
        index.add_path(&path)?;
        let oid = index.write_tree()?;
        let tree = self.inner.find_tree(oid)?;
        let sig = Signature::now("Cepler", "bot@cepler.io")?;
        self.inner.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &format!(
                "[cepler] Updated {} state",
                path.file_stem().unwrap().to_str().unwrap()
            ),
            &tree,
            &[&self.head_commit()],
        )?;
        let mut checkout = CheckoutBuilder::new();
        checkout.path(path);
        self.inner.checkout_index(None, Some(&mut checkout))?;
        Ok(())
    }

    pub fn head_files(
        &self,
        filters: &[String],
        ignore_files: Vec<Pattern>,
    ) -> impl Iterator<Item = PathBuf> + '_ {
        let mut opts = MatchOptions::new();
        opts.require_literal_leading_dot = true;
        let files: Vec<_> = filters
            .iter()
            .map(move |files| glob_with(&files, opts).expect("Couldn't resolve glob"))
            .flatten()
            .map(|res| res.expect("Couldn't list file"))
            .collect();
        let repo = Self::open().expect("Couldn't re-open repo");
        files.into_iter().filter(move |file| {
            repo.is_trackable_file(&file)
                && !ignore_files
                    .iter()
                    .any(|p| p.matches(file.to_str().unwrap()))
        })
    }

    fn is_trackable_file(&self, file: &PathBuf) -> bool {
        let path = file.as_path();
        if self.inner.status_file(path).is_err() {
            return false;
        }
        !self
            .inner
            .status_should_ignore(path)
            .expect("Cannot check ignore status")
    }

    pub fn is_file_dirty(&self, file: &PathBuf) -> Result<bool> {
        Ok(!self.inner.status_file(file.as_path())?.is_empty())
    }

    pub fn head_commit_hash(&self) -> Result<CommitHash> {
        Ok(CommitHash(self.head_oid().to_string()))
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

    pub fn checkout_head(
        &self,
        filters: Option<&[String]>,
        ignore_files: Vec<Pattern>,
    ) -> Result<()> {
        self.inner
            .reset(self.head_commit().as_object(), ResetType::Hard, None)?;
        if let Some(filters) = filters {
            let mut checkout = CheckoutBuilder::new();
            checkout.force();
            for path in self.head_files(filters, ignore_files.clone()) {
                checkout.path(path);
            }

            for path in glob("**/*").expect("List all files") {
                let path = path.expect("Get file");
                if self.is_trackable_file(&path) {
                    let path = path.as_path();
                    if !ignore_files.iter().any(|p| p.matches_path(path)) && path.is_file() {
                        std::fs::remove_file(path).expect("Couldn't remove file");
                    }
                }
            }
            if !filters.is_empty() {
                self.inner
                    .checkout_head(Some(&mut checkout))
                    .expect("Couldn't checkout");
            }
        }
        Ok(())
    }

    pub fn find_last_changed_commit(
        &self,
        files: Vec<&Path>,
        deleted: Vec<&Path>,
    ) -> Result<(CommitHash, String)> {
        let head_commit = self.head_commit();
        let head_tree = head_commit.tree().context("Couldn't resolve tree")?;
        let mut set = HashSet::new();
        let mut queue = VecDeque::new();
        set.insert(head_commit.id());
        queue.push_back(head_commit);

        loop {
            let commit = queue.pop_front().unwrap();
            let mut go = true;
            for parent in commit.parents() {
                let mut found_diff = false;
                let parent_tree = parent.tree().expect("Couldn't get tree");
                for file in files.iter() {
                    let target = head_tree.get_path(file).context("Missing file")?;
                    if let Ok(tree) = parent_tree.get_path(file) {
                        if tree.id() != target.id() {
                            found_diff = true;
                            break;
                        }
                    } else {
                        found_diff = true;
                        break;
                    }
                }
                if !found_diff {
                    for file in deleted.iter() {
                        if parent_tree.get_path(file).is_ok() {
                            found_diff = true;
                            break;
                        }
                    }
                }
                if !found_diff && set.insert(parent.id()) {
                    queue.push_back(parent);
                }
                if found_diff {
                    go = false;
                    break;
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

    fn head_commit(&self) -> Commit<'_> {
        self.inner.head().unwrap().peel_to_commit().unwrap()
    }

    fn head_oid(&self) -> Oid {
        self.inner.head().unwrap().peel_to_commit().unwrap().id()
    }
}

fn remote_callbacks(key: String) -> Result<RemoteCallbacks<'static>> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, username_from_url, _allowed_types| {
        Cred::ssh_key_from_memory(username_from_url.unwrap(), None, &key, None)
    });
    Ok(callbacks)
}
