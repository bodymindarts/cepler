use super::config::{MATCH_OPTIONS, default_scope};
use anyhow::{Context, Result};
use git2::{
    BranchType, CertificateCheckStatus, Commit, Cred, MergeOptions, Object, ObjectType, Oid,
    PushOptions, RebaseOptions, RemoteCallbacks, Repository, ResetType, Signature, TreeWalkMode,
    TreeWalkResult, build::CheckoutBuilder,
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

        // Skip libgit2's implicit working-tree checkout. For repos with
        // many files it dominates clone wall-clock time (~2-3 minutes on
        // the volcano-qa state repo) and emits no progress, so it looks
        // like a hang. Callers materialise just the files they actually
        // need via `checkout_paths` (concourse `in`/`check`) or
        // `checkout_head` (CLI), which is orders of magnitude faster than
        // writing every blob in HEAD.
        let mut no_checkout = git2::build::CheckoutBuilder::new();
        no_checkout.dry_run();

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(&branch);
        builder.with_checkout(no_checkout);

        let t = std::time::Instant::now();
        let inner = builder.clone(&url, Path::new(&dir))?;
        eprintln!(
            "[cepler-perf] RepoBuilder::clone (fetch + pack index): {:.2}s",
            t.elapsed().as_secs_f64()
        );

        // libgit2 short-circuits both working-tree AND index population
        // when checkout_strategy == GIT_CHECKOUT_NONE (see clone.c
        // `should_checkout`). The index needs to be in sync with HEAD so
        // that a later `index.write_tree()` from `commit_state_file` on
        // the `out` step produces a tree that still includes everything
        // inherited from HEAD — without this step `out` would commit a
        // tree containing only the new state file, deleting the rest of
        // the repo. ResetType::Mixed fills the index from HEAD without
        // touching the working tree.
        let t = std::time::Instant::now();
        {
            let head_commit = inner.head()?.peel_to_commit()?;
            let head_obj = head_commit.into_object();
            inner.reset(&head_obj, ResetType::Mixed, None)?;
        }
        eprintln!(
            "[cepler-perf] index sync (Mixed reset from HEAD): {:.2}s",
            t.elapsed().as_secs_f64()
        );

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

    pub fn push(&self, config: GitConfig) -> Result<bool> {
        const MAX_PUSH_ATTEMPTS: usize = 5;
        let GitConfig {
            branch,
            private_key,
            ..
        } = config;

        let mut attempt = 0;
        loop {
            attempt += 1;
            // Skip the pre-push fetch on the first attempt: the workspace was
            // freshly cloned/pulled milliseconds ago by `in`/`check`, so
            // refreshing origin again is wasted bandwidth in the happy path.
            // On a `NotFastForward` retry we fall back to fetching so the
            // rebase has up-to-date upstream.
            let refresh_remote = attempt > 1;
            match self.try_push(&branch, &private_key, refresh_remote) {
                Ok(pushed) => return Ok(pushed),
                Err(e) => {
                    let non_fast_forward = e
                        .downcast_ref::<git2::Error>()
                        .map(|git_err| git_err.code() == git2::ErrorCode::NotFastForward)
                        .unwrap_or(false);
                    if non_fast_forward && attempt < MAX_PUSH_ATTEMPTS {
                        eprintln!(
                            "Push rejected because the remote moved; re-fetching and retrying ({}/{})",
                            attempt, MAX_PUSH_ATTEMPTS
                        );
                        std::thread::sleep(std::time::Duration::from_millis(200 * attempt as u64));
                        continue;
                    }
                    return Err(e).context("Couldn't push to remote");
                }
            }
        }
    }

    fn try_push(&self, branch: &str, private_key: &str, refresh_remote: bool) -> Result<bool> {
        let mut remote = self.inner.find_remote("origin")?;
        if refresh_remote {
            let callbacks = remote_callbacks(private_key.to_string());
            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(callbacks);
            remote
                .fetch(&[branch], Some(&mut fo), None)
                .context("Couldn't fetch origin")?;
        }

        let annotated_head = self
            .inner
            .reference_to_annotated_commit(&self.inner.head()?)
            .context("Couldn't find head reference")?;

        let head_commit = match self.inner.find_branch(branch, BranchType::Local) {
            Ok(local) if local.is_head() => annotated_head,
            _ => {
                let branch_ref = self
                    .inner
                    .branch_from_annotated_commit(branch, &annotated_head, true)
                    .context("Could not create local branch")?;
                self.inner.reference_to_annotated_commit(branch_ref.get())?
            }
        };

        let remote_ref = self
            .inner
            .resolve_reference_from_short_name(&format!("origin/{}", branch))
            .context("Couldn't resolve remote branch")?;
        let remote_commit = self
            .inner
            .reference_to_annotated_commit(&remote_ref)
            .context("Couldn't get remote commit")?;

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

        let mut n_applied = 0;
        while let Some(_) = rebase.next() {
            let res = rebase.commit(None, &sig, None);
            if matches!(res.as_ref(), Err(e) if e.code() == git2::ErrorCode::Applied) {
                continue;
            }
            n_applied += 1;
            res.context("Couldn't commit rebase")?;
        }
        rebase.finish(None).context("Couldn't finish rebase")?;

        if n_applied > 0 {
            let mut push_options = PushOptions::new();
            push_options.remote_callbacks(remote_callbacks(private_key.to_string()));
            let refname = head_commit
                .refname()
                .context("Annotated commit has no reference name")?;
            remote.push(
                &[format!("{}:{}", refname, refname)],
                Some(&mut push_options),
            )?;
            Ok(true)
        } else {
            Ok(false)
        }
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

    pub fn commit_state_file(&self, scope: &str, file_name: String) -> Result<()> {
        let path = Path::new(&file_name);
        let mut index = self.inner.index()?;
        index.add_path(path)?;
        let oid = index.write_tree()?;
        let tree = self.inner.find_tree(oid)?;
        let sig = Signature::now("Cepler", "bot@cepler.io")?;

        let head_commit = self.inner.head().unwrap().peel_to_commit().unwrap();
        let msg = if scope != default_scope() {
            format!(
                "ci(cepler): Updated '{}' state in '{}'",
                scope,
                path.file_stem().unwrap().to_str().unwrap()
            )
        } else {
            format!(
                "ci(cepler): Updated '{}' state",
                path.file_stem().unwrap().to_str().unwrap()
            )
        };
        self.inner
            .commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[&head_commit])?;
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
            if !matches!(entry.kind(), Some(ObjectType::Blob)) {
                return TreeWalkResult::Ok;
            }
            if let Err(e) = f(FileHash(entry.id().to_string()), path) {
                ret = Err(e);
                return TreeWalkResult::Abort;
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

    pub fn head_commit_summary(&self) -> Result<(CommitHash, String)> {
        let commit = self.inner.head().unwrap().peel_to_commit().unwrap();
        Ok((
            CommitHash(commit.id().to_string()),
            commit.summary().expect("Couldn't get summary").to_string(),
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
                if !ignore_files.iter().any(check)
                    && path.is_file()
                    && (clean || globs.iter().any(check))
                {
                    std::fs::remove_file(path).expect("Couldn't remove file");
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

    /// Selectively materialise specific paths from HEAD into the working
    /// tree. Patterns follow libgit2's fnmatch-style semantics — `*`
    /// inside a single directory component, no `**` recursion — so a
    /// state directory is expressed as `dir/*` (the state files are flat
    /// under it). Used by the concourse `in`/`check` flows after a
    /// no-checkout clone to populate just the files cepler reads from
    /// disk (config + state dir + optional gates file).
    ///
    /// The index is left untouched (`update_index(false)`) since
    /// `Repo::clone` already populated it in full via the Mixed reset —
    /// a partial WT checkout shouldn't be allowed to shrink that record.
    pub fn checkout_paths<I, S>(&self, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.update_index(false);
        let mut any = false;
        for path in paths {
            checkout.path(path.as_ref());
            any = true;
        }
        if !any {
            return Ok(());
        }
        let head_commit = self.inner.head()?.peel_to_commit()?;
        let head_obj = head_commit.into_object();
        self.inner.checkout_tree(&head_obj, Some(&mut checkout))?;
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

    fn gate_object(&self) -> Object<'_> {
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
    // libgit2 1.7+ (pulled in by git2 0.20) rejects unknown SSH host keys
    // with `invalid or unknown remote ssh hostkey; class=Ssh (23); code=-17`
    // unless the caller installs a certificate_check callback. The
    // pre-bump cepler (git2 0.13 / libgit2 1.3.x) silently accepted any
    // host key — concourse pipelines run against a known git remote and
    // the user's SSH private key already authenticates the connection,
    // so restore the prior behaviour by explicitly accepting the host
    // certificate. Returning `CertificatePassthrough` would defer to
    // libgit2's built-in check, which without a known_hosts file at a
    // path libgit2 looks for will hard-reject every host.
    callbacks.certificate_check(|_cert, _host| Ok(CertificateCheckStatus::CertificateOk));
    callbacks
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!(
            "cepler-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stage_and_commit(repo: &Repository, msg: &str) -> Oid {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        let parents: Vec<Commit> = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap()
    }

    fn push_branch(repo: &Repository, branch: &str) {
        let mut remote = repo.find_remote("origin").unwrap();
        remote
            .push(&[format!("refs/heads/{0}:refs/heads/{0}", branch)], None)
            .unwrap();
    }

    #[test]
    fn push_rebases_state_commit_onto_concurrent_remote_commit() {
        let base = unique_tmp_dir("push");
        let bare_path = base.join("remote.git");
        let work_a = base.join("work_a");
        let work_b = base.join("work_b");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let repo_a = Repository::init(&work_a).unwrap();
        repo_a.remote("origin", &bare_url).unwrap();
        std::fs::write(work_a.join("file.txt"), "v1").unwrap();
        stage_and_commit(&repo_a, "initial commit");
        let branch = repo_a.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&repo_a, &branch);

        let repo_b = Repository::clone(&bare_url, &work_b).unwrap();
        std::fs::write(work_b.join("other.txt"), "concurrent").unwrap();
        let concurrent = stage_and_commit(&repo_b, "concurrent writer commit");
        push_branch(&repo_b, &branch);

        std::fs::write(work_a.join("state.txt"), "cepler state").unwrap();
        stage_and_commit(&repo_a, "ci(cepler): Updated state");

        let repo = Repo {
            inner: repo_a,
            gate: None,
        };
        let config = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: work_a.to_str().unwrap().to_string(),
        };

        let pushed = repo.push(config).expect("push should succeed after rebase");
        assert!(pushed, "expected the state commit to be pushed");

        let verify = Repository::clone(&bare_url, base.join("verify")).unwrap();
        let tip = verify.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(tip.summary().unwrap(), "ci(cepler): Updated state");
        assert_eq!(
            tip.parent(0).unwrap().id(),
            concurrent,
            "state commit must be rebased onto the concurrent commit"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn first_push_attempt_skips_remote_fetch() {
        // Build a remote with two commits and a clone that only knows the
        // first one. If `try_push` were still doing a pre-push fetch, our
        // local view would jump forward and the push would succeed.
        // Skipping the fetch means we still think origin is at commit #1,
        // so a non-fast-forward push fails (without ever touching the network).
        let base = unique_tmp_dir("first-push-no-fetch");
        let bare_path = base.join("remote.git");
        let work_a = base.join("work_a");
        let work_b = base.join("work_b");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let repo_a = Repository::init(&work_a).unwrap();
        repo_a.remote("origin", &bare_url).unwrap();
        std::fs::write(work_a.join("file.txt"), "v1").unwrap();
        stage_and_commit(&repo_a, "initial commit");
        let branch = repo_a.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&repo_a, &branch);

        // Clone B from origin (sees commit #1).
        let repo_b = Repository::clone(&bare_url, &work_b).unwrap();

        // Worker A pushes commit #2 to origin out-of-band.
        std::fs::write(work_a.join("file.txt"), "v2").unwrap();
        stage_and_commit(&repo_a, "out of band commit");
        push_branch(&repo_a, &branch);

        // Worker B records its own cepler state on top of its stale view…
        std::fs::write(work_b.join("state.txt"), "cepler state").unwrap();
        stage_and_commit(&repo_b, "ci(cepler): Updated state");

        // Point an unreachable URL to force a network error if any
        // attempt re-fetches. The first attempt should still succeed at the
        // rebase step (it never fetches) and only the push itself can fail.
        let unreachable = "ssh://invalid.invalid/does-not-exist.git";
        repo_b.remote_set_url("origin", unreachable).unwrap();

        let repo = Repo {
            inner: repo_b,
            gate: None,
        };

        // Single-attempt try_push with refresh_remote = false. Skipping the
        // fetch must not error out before reaching the push (the rebase uses
        // the stale local origin/<branch> ref). The push itself will fail
        // because the URL is bogus, which is fine — we're asserting we made
        // it past the fetch.
        let result = repo.try_push(&branch, "", false);
        let err = result.expect_err("push to unreachable remote should fail");
        let msg = format!("{:#}", err);
        assert!(
            !msg.contains("Couldn't fetch origin"),
            "first attempt must skip pre-push fetch, got: {msg}"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn clone_skips_working_tree_checkout_but_populates_index() {
        // The whole point of switching `Repo::clone` to a no-checkout
        // clone is that the working tree stays empty (zero per-file
        // syscalls during clone for repos with thousands of files) while
        // the index stays fully in sync with HEAD — otherwise the next
        // `commit_state_file` would build a tree containing only the
        // newly-added state file, deleting everything else inherited
        // from HEAD.
        let base = unique_tmp_dir("clone-no-checkout");
        let bare_path = base.join("remote.git");
        let seed_path = base.join("seed");
        let dest_path = base.join("dest");
        let bare_url = bare_path.to_str().unwrap().to_string();

        // Build a remote with a few tracked files in HEAD.
        Repository::init_bare(&bare_path).unwrap();
        let seed = Repository::init(&seed_path).unwrap();
        seed.remote("origin", &bare_url).unwrap();
        std::fs::write(seed_path.join("a.txt"), "alpha").unwrap();
        std::fs::write(seed_path.join("b.txt"), "beta").unwrap();
        std::fs::create_dir_all(seed_path.join("nested")).unwrap();
        std::fs::write(seed_path.join("nested/c.txt"), "gamma").unwrap();
        stage_and_commit(&seed, "seed commit");
        let branch = seed.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&seed, &branch);

        let conf = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: dest_path.to_str().unwrap().to_string(),
        };
        let repo = Repo::clone(conf).expect("clone should succeed");

        // Working tree is empty — the only entry under `dest/` should be
        // the `.git` directory.
        let mut wd_entries: Vec<String> = std::fs::read_dir(&dest_path)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        wd_entries.sort();
        assert_eq!(
            wd_entries,
            vec![".git".to_string()],
            "no-checkout clone must leave the working tree empty"
        );

        // Index is fully populated — every blob from HEAD's tree should
        // be in the index so a downstream `write_tree()` would still
        // produce the HEAD tree (plus any additions).
        let mut index = repo.inner.index().unwrap();
        let entry_paths: std::collections::BTreeSet<String> = (0..index.len())
            .map(|i| String::from_utf8(index.get(i).unwrap().path).expect("utf8 index path"))
            .collect();
        let expected: std::collections::BTreeSet<String> = ["a.txt", "b.txt", "nested/c.txt"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            entry_paths, expected,
            "index must include every HEAD entry after the Mixed-reset",
        );

        // `write_tree` from the just-cloned index should reproduce HEAD's
        // tree exactly — this is what makes `commit_state_file` safe.
        let written = index.write_tree_to(&repo.inner).unwrap();
        let head_tree = repo
            .inner
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap()
            .id();
        assert_eq!(
            written, head_tree,
            "index.write_tree() must reproduce HEAD tree post-clone",
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn checkout_paths_materialises_only_requested_files() {
        // The concourse `in`/`check` flows pair a no-checkout clone with
        // a targeted `checkout_paths` for just the files cepler reads
        // from disk. Verify that the helper writes exactly those paths
        // (single files + glob patterns) and nothing else.
        let base = unique_tmp_dir("checkout-paths");
        let bare_path = base.join("remote.git");
        let seed_path = base.join("seed");
        let dest_path = base.join("dest");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let seed = Repository::init(&seed_path).unwrap();
        seed.remote("origin", &bare_url).unwrap();
        std::fs::write(seed_path.join("cepler.yml"), "deployment: demo").unwrap();
        std::fs::create_dir_all(seed_path.join(".cepler/demo")).unwrap();
        std::fs::write(seed_path.join(".cepler/demo/staging.state"), "v1").unwrap();
        std::fs::write(seed_path.join(".cepler/demo/prod.state"), "v2").unwrap();
        std::fs::write(seed_path.join("other.txt"), "should stay unchecked-out").unwrap();
        stage_and_commit(&seed, "seed");
        let branch = seed.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&seed, &branch);

        let conf = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: dest_path.to_str().unwrap().to_string(),
        };
        let repo = Repo::clone(conf).expect("clone");

        repo.checkout_paths(["cepler.yml", ".cepler/demo/*"])
            .expect("checkout_paths");

        let on_disk = |p: &str| dest_path.join(p).is_file();
        assert!(on_disk("cepler.yml"), "config must be materialised");
        assert!(
            on_disk(".cepler/demo/staging.state"),
            "state files matching the glob must be materialised"
        );
        assert!(on_disk(".cepler/demo/prod.state"));
        assert!(
            !on_disk("other.txt"),
            "files not matching any requested path must stay unchecked-out"
        );

        std::fs::remove_dir_all(&base).ok();
    }
}
