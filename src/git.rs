use git2::{build::CheckoutBuilder, Commit, ObjectType, Oid, Repository, ResetType, Signature};
use glob::*;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileHash(String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitHash(String);

pub fn hash_file<P: AsRef<Path>>(file: P) -> FileHash {
    FileHash(
        Oid::hash_file(ObjectType::Blob, file)
            .expect("Couldn't hash object")
            .to_string(),
    )
}

pub struct Repo {
    inner: Repository,
}

impl Repo {
    pub fn open() -> Result<Self, git2::Error> {
        Ok(Self {
            inner: Repository::open_from_env()?,
        })
    }

    pub fn commit_state_file(&self, file_name: String) -> Result<(), git2::Error> {
        let path = Path::new(&file_name);
        let mut index = self.inner.index()?;
        index.clear()?;
        index.add_path(&path)?;
        let oid = index.write_tree()?;
        let tree = self.inner.find_tree(oid)?;
        let sig = Signature::now("Casper", "bot@casper.io")?;
        self.inner.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &format!(
                "[cepler] State for {} recorder",
                path.file_stem().unwrap().to_str().unwrap()
            ),
            &tree,
            &[&self.head_commit()],
        )?;
        Ok(())
    }

    pub fn head_files(&self, filters: &[String]) -> impl Iterator<Item = PathBuf> + '_ {
        let mut opts = MatchOptions::new();
        opts.require_literal_leading_dot = true;
        let files: Vec<_> = filters
            .iter()
            .map(move |files| glob_with(&files, opts).expect("Couldn't resolve glob"))
            .flatten()
            .map(|res| res.expect("Couldn't list file"))
            .collect();
        let repo = Self::open().expect("Couldn't re-open repo");
        files.into_iter().filter_map(move |file| {
            if repo.is_trackable_file(&file) {
                Some(file)
            } else {
                None
            }
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

    pub fn is_file_dirty(&self, file: &PathBuf) -> Result<bool, git2::Error> {
        Ok(!self.inner.status_file(file.as_path())?.is_empty())
    }

    pub fn head_commit_hash(&self) -> Result<CommitHash, git2::Error> {
        Ok(CommitHash(self.head_oid().to_string()))
    }

    pub fn checkout_file_from<'a>(
        &self,
        path: &str,
        commit: &CommitHash,
    ) -> Result<(), git2::Error> {
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
        ignore_files: Vec<&str>,
    ) -> Result<(), git2::Error> {
        let mut checkout = CheckoutBuilder::new();
        self.inner.reset(
            self.head_commit().as_object(),
            ResetType::Hard,
            Some(&mut checkout),
        )?;
        if let Some(filters) = filters {
            let mut ignore_os_files: HashSet<_> = ignore_files.iter().map(OsStr::new).collect();
            ignore_os_files.insert(OsStr::new(".git"));

            let mut checkout = CheckoutBuilder::new();
            checkout.force();
            for path in self.head_files(filters) {
                checkout.path(path);
            }

            for path in glob("*").expect("List all files") {
                let path = path.expect("Get file");
                if let Some(name) = path.file_name() {
                    if !ignore_os_files.contains(name) {
                        if path.as_path().is_dir() {
                            std::fs::remove_dir_all(path).expect("Couldn't remove file");
                        } else {
                            std::fs::remove_file(path).expect("Couldn't remove file");
                        }
                    }
                }
            }
            self.inner
                .checkout_head(Some(&mut checkout))
                .expect("Couldn't checkout");
        }
        Ok(())
    }

    pub fn find_last_changed_commit(&self, file: &Path) -> (CommitHash, String) {
        let commit = self.head_commit();
        let target = commit
            .tree()
            .expect("Couldn't resolve tree")
            .get_path(file)
            .expect("Couldn't get path");
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
                return (
                    CommitHash(commit.id().to_string()),
                    commit.summary().expect("Couldn't get summary").to_string(),
                );
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
