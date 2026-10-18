//! The few git commands ccs runs itself, around the worktrees Claude makes for sessions
//! (`claude --worktree`). Without git, every directory is outside a repository.

use std::{
    fs::OpenOptions,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process::Command,
};

/// What keeps Claude's worktrees out of `git status` in the main checkout.
const EXCLUDE: &str = "/.claude/worktrees/";

/// Runs `git -C dir args`, for its output.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|err| format!("running git: {err}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned())
    } else {
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The commit checked out in `dir`. `None` outside a repository, or before its first commit.
pub fn head(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]).ok()
}

/// The branch checked out in `dir`. `None` outside a repository, or on a detached HEAD.
pub fn branch(dir: &Path) -> Option<String> {
    git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
}

/// The top of the working tree `dir` is in.
pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(PathBuf::from)
}

/// The changes in `dir` not committed yet, one per line as `git status --short` lists them.
pub fn changes(dir: &Path) -> Result<Vec<String>, String> {
    let status = git(dir, &["status", "--porcelain"])?;
    Ok(status.lines().map(str::to_owned).collect())
}

/// How many commits HEAD in `dir` has that `branch` doesn't.
pub fn ahead(dir: &Path, branch: &str) -> Result<usize, String> {
    let range = format!("{branch}..HEAD");
    let count = git(dir, &["rev-list", "--count", &range])?;
    count
        .parse()
        .map_err(|err| format!("git rev-list --count: {err}"))
}

/// Some file in `dir` is in conflict, after a merge or a rebase that stopped on it.
pub fn unmerged(dir: &Path) -> bool {
    git(dir, &["ls-files", "--unmerged"]).is_ok_and(|files| !files.is_empty())
}

/// A merge in `dir` is under way: it stopped, and isn't committed or aborted yet.
pub fn merging(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"]).is_ok()
}

/// `commit` is in `branch`.
pub fn contains(dir: &Path, commit: &str, branch: &str) -> bool {
    git(dir, &["merge-base", "--is-ancestor", commit, branch]).is_ok()
}

/// The branches merged into `target` in `repo`, as its reflog has them: by `git merge`, fast
/// forward or not, or by the commit that finished a merge that stopped on conflicts. Only merges
/// made here, as far back as the reflog goes; they are found even after the branch is deleted.
pub fn merged_into(repo: &Path, target: &str) -> Result<Vec<String>, String> {
    let target = format!("refs/heads/{target}");
    let log = git(repo, &["reflog", "show", "--format=%gs", &target])?;
    Ok(log
        .lines()
        .filter_map(merged_branch)
        .map(str::to_owned)
        .collect())
}

/// The branch a reflog entry merged: `merge <branch>: …`, or
/// `commit (merge): Merge branch '<branch>'…`.
fn merged_branch(entry: &str) -> Option<&str> {
    if let Some(rest) = entry.strip_prefix("merge ") {
        return rest.split_once(':').map(|(branch, _)| branch);
    }
    let rest = entry.strip_prefix("commit (merge): Merge branch '")?;
    rest.split_once('\'').map(|(branch, _)| branch)
}

/// The branches in `repo` with commits `target` doesn't have.
pub fn not_in(repo: &Path, target: &str) -> Result<Vec<String>, String> {
    let list = git(
        repo,
        &["branch", "--format=%(refname:short)", "--no-merged", target],
    )?;
    Ok(list.lines().map(str::to_owned).collect())
}

/// Keeps the worktrees Claude makes out of `git status` in the main checkout of `repo`, in the
/// repository's own `info/exclude`, which is not committed.
pub fn exclude_worktrees(repo: &Path) -> Result<(), String> {
    let path = git(
        repo,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ],
    )?;
    let path = PathBuf::from(path);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("reading {}: {err}", path.display())),
    };
    if text.lines().any(|line| line.trim() == EXCLUDE) {
        return Ok(());
    }
    let newline = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let write = || {
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(file, "{newline}{EXCLUDE}")
    };
    write().map_err(|err| format!("writing {}: {err}", path.display()))
}

/// Removes the worktree `name` that Claude made in `repo`, and its branch `worktree-<name>`,
/// if nothing in it would be lost: no changes, and no commit that no other branch has. Either
/// way it is unlocked, as Claude does when you keep one: the Claude that locked it is gone.
/// Whether it was removed; it is kept on a detached HEAD, and is not there after Claude removed
/// it itself.
pub fn remove_if_unused(repo: &Path, name: &str) -> Result<bool, String> {
    let list = git(repo, &["worktree", "list", "--porcelain"])?;
    let Some((path, branch)) = find_worktree(&list, name) else {
        return Ok(false);
    };
    let dir = path.to_str().ok_or("the worktree path is not UTF-8")?;
    // Fails when it isn't locked, which is as good.
    let _ = git(repo, &["worktree", "unlock", dir]);
    let Some(branch) = branch else {
        return Ok(false);
    };
    if !path.is_dir() {
        return Ok(false);
    }
    if !git(&path, &["status", "--porcelain"])?.is_empty() {
        return Ok(false);
    }
    let others = format!("--exclude={branch}");
    let own = git(
        &path,
        &[
            "rev-list",
            "-n1",
            "HEAD",
            "--not",
            &others,
            "--branches",
            "--remotes",
        ],
    )?;
    if !own.is_empty() {
        return Ok(false);
    }
    git(repo, &["worktree", "remove", "--force", dir])?;
    if branch == format!("worktree-{name}") {
        git(repo, &["branch", "-D", &branch])?;
    }
    Ok(true)
}

/// The path and branch of the worktree `name` under `.claude/worktrees` in the output of
/// `git worktree list --porcelain`. The branch is `None` on a detached HEAD.
fn find_worktree(list: &str, name: &str) -> Option<(PathBuf, Option<String>)> {
    let end = Path::new(".claude/worktrees").join(name);
    list.split("\n\n").find_map(|block| {
        let path = PathBuf::from(block.lines().next()?.strip_prefix("worktree ")?);
        if !path.ends_with(&end) {
            return None;
        }
        let branch = block
            .lines()
            .find_map(|line| line.strip_prefix("branch refs/heads/"))
            .map(str::to_owned);
        Some((path, branch))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A new repository with one commit.
    fn repo() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ccs-git-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        run(&dir, &["add", "a.txt"]);
        commit(&dir);
        dir
    }

    fn run(dir: &Path, args: &[&str]) {
        git(dir, args).unwrap();
    }

    fn commit(dir: &Path) {
        run(
            dir,
            &[
                "-c",
                "user.name=ccs",
                "-c",
                "user.email=ccs@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qam",
                "change",
            ],
        );
    }

    /// The worktree `name` as Claude makes it, locked as a running Claude holds it.
    fn worktree(repo: &Path, name: &str) -> PathBuf {
        let path = repo.join(".claude/worktrees").join(name);
        let dir = path.to_str().unwrap();
        let branch = format!("worktree-{name}");
        run(repo, &["worktree", "add", "-q", "-b", &branch, dir]);
        run(
            repo,
            &["worktree", "lock", "--reason", "claude session", dir],
        );
        path
    }

    fn branches(repo: &Path) -> String {
        git(repo, &["branch", "--format=%(refname:short)"]).unwrap()
    }

    /// Runs `git args` in `dir` as someone who can commit.
    fn as_user(dir: &Path, args: &[&str]) -> Result<String, String> {
        let user = [
            "-c",
            "user.name=ccs",
            "-c",
            "user.email=ccs@example.com",
            "-c",
            "commit.gpgsign=false",
        ];
        git(dir, &[&user[..], args].concat())
    }

    /// Commits the new file `name` in `dir`.
    fn commit_file(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "new\n").unwrap();
        run(dir, &["add", name]);
        commit(dir);
    }

    #[test]
    fn reads_head_and_branch() {
        let repo = repo();
        assert_eq!(head(&repo).unwrap().len(), 40);
        assert_eq!(branch(&repo).as_deref(), Some("main"));
        std::fs::create_dir_all(repo.join("src")).unwrap();
        assert_eq!(
            toplevel(&repo.join("src")).unwrap().canonicalize().unwrap(),
            repo.canonicalize().unwrap()
        );
        let empty = repo.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        run(&empty, &["init", "-q"]);
        // A repository with no commit yet has no HEAD to branch from.
        assert_eq!(head(&empty), None);
        assert_eq!(head(Path::new("/")), None);
        assert_eq!(toplevel(Path::new("/")), None);
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn removes_a_worktree_with_nothing_in_it() {
        let repo = repo();
        let path = worktree(&repo, "ccs-1");
        // Found from the main checkout, and only by its own name.
        assert!(!remove_if_unused(&repo, "ccs").unwrap());
        assert!(remove_if_unused(&repo, "ccs-1").unwrap());
        assert!(!path.exists());
        assert_eq!(branches(&repo), "main");
        // Gone already, as after Claude removed it.
        assert!(!remove_if_unused(&repo, "ccs-1").unwrap());
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn keeps_a_worktree_with_changes_or_commits() {
        let repo = repo();
        let changed = worktree(&repo, "changed");
        std::fs::write(changed.join("a.txt"), "b\n").unwrap();
        let new = worktree(&repo, "new");
        std::fs::write(new.join("b.txt"), "b\n").unwrap();
        let committed = worktree(&repo, "committed");
        std::fs::write(committed.join("a.txt"), "c\n").unwrap();
        commit(&committed);
        for name in ["changed", "new", "committed"] {
            assert!(!remove_if_unused(&repo, name).unwrap(), "{name}");
        }
        assert!(changed.exists() && new.exists() && committed.exists());
        // Kept, but no longer locked by the Claude that is gone.
        let list = git(&repo, &["worktree", "list", "--porcelain"]).unwrap();
        assert!(!list.contains("locked"), "{list}");
        assert_eq!(
            branches(&repo),
            "main\nworktree-changed\nworktree-committed\nworktree-new"
        );
        // A commit another branch has too is not lost.
        run(&repo, &["branch", "keep", "worktree-committed"]);
        assert!(remove_if_unused(&repo, "committed").unwrap());
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn tells_what_a_merge_would_take_and_where_it_stopped() {
        let repo = repo();
        let tree = worktree(&repo, "ccs-1");
        assert_eq!(changes(&tree).unwrap(), Vec::<String>::new());
        assert_eq!(ahead(&tree, "main").unwrap(), 0);
        std::fs::write(tree.join("a.txt"), "tree\n").unwrap();
        std::fs::write(tree.join("b.txt"), "b\n").unwrap();
        assert_eq!(changes(&tree).unwrap(), [" M a.txt", "?? b.txt"]);
        commit(&tree);
        let tip = head(&tree).unwrap();
        assert_eq!(ahead(&tree, "main").unwrap(), 1);
        assert!(!contains(&repo, &tip, "main"));

        // The same line changed on main: merging the worktree's branch there stops on it.
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        commit(&repo);
        assert!(!unmerged(&repo) && !merging(&repo));
        assert!(git(&repo, &["merge", "--no-edit", "worktree-ccs-1"]).is_err());
        assert!(unmerged(&repo) && merging(&repo));
        assert!(!unmerged(&tree));
        // Fixed and committed, the worktree's commit is in main.
        std::fs::write(repo.join("a.txt"), "both\n").unwrap();
        run(&repo, &["add", "a.txt"]);
        commit(&repo);
        assert!(!unmerged(&repo) && !merging(&repo));
        assert!(contains(&repo, &tip, "main"));
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn finds_merged_branches_in_the_reflog() {
        let repo = repo();
        // Fast forward, and gone with its worktree after, as after leader m.
        let ff = worktree(&repo, "ff");
        commit_file(&ff, "ff.txt");
        as_user(&repo, &["merge", "--no-edit", "worktree-ff"]).unwrap();
        assert!(remove_if_unused(&repo, "ff").unwrap());
        // A merge commit, main having moved on.
        let ort = worktree(&repo, "ort");
        commit_file(&ort, "ort.txt");
        commit_file(&repo, "main.txt");
        as_user(&repo, &["merge", "--no-edit", "worktree-ort"]).unwrap();
        // Stopped on conflicts, then finished as Claude is asked to.
        let fixed = worktree(&repo, "fixed");
        std::fs::write(fixed.join("a.txt"), "fixed\n").unwrap();
        commit(&fixed);
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        commit(&repo);
        assert!(as_user(&repo, &["merge", "--no-edit", "worktree-fixed"]).is_err());
        std::fs::write(repo.join("a.txt"), "both\n").unwrap();
        run(&repo, &["add", "a.txt"]);
        as_user(&repo, &["commit", "--no-edit"]).unwrap();
        // Not merged, and nothing to merge.
        commit_file(&worktree(&repo, "open"), "open.txt");
        worktree(&repo, "fresh");

        assert_eq!(
            merged_into(&repo, "main").unwrap(),
            ["worktree-fixed", "worktree-ort", "worktree-ff"]
        );
        assert_eq!(not_in(&repo, "main").unwrap(), ["worktree-open"]);
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn reads_the_branch_a_reflog_entry_merged() {
        for (entry, branch) in [
            ("merge worktree-ccs-1: Fast-forward", Some("worktree-ccs-1")),
            (
                "merge worktree-ccs-1: Merge made by the 'ort' strategy.",
                Some("worktree-ccs-1"),
            ),
            (
                "commit (merge): Merge branch 'worktree-ccs-1'",
                Some("worktree-ccs-1"),
            ),
            (
                "commit (merge): Merge branch 'worktree-ccs-1' into dev",
                Some("worktree-ccs-1"),
            ),
            ("commit (merge): change", None),
            ("commit: change", None),
            ("commit (initial): first commit", None),
        ] {
            assert_eq!(merged_branch(entry), branch, "{entry}");
        }
    }

    #[test]
    fn excludes_the_worktrees_once() {
        let repo = repo();
        let exclude = repo.join(".git/info/exclude");
        std::fs::write(&exclude, "*.log").unwrap();
        exclude_worktrees(&repo).unwrap();
        // From a subdirectory, and again: nothing more.
        std::fs::create_dir_all(repo.join("src")).unwrap();
        exclude_worktrees(&repo.join("src")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&exclude).unwrap(),
            "*.log\n/.claude/worktrees/\n"
        );
        worktree(&repo, "ccs-1");
        assert_eq!(git(&repo, &["status", "--porcelain"]).unwrap(), "");
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
