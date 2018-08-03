use dispatch::{communicate, dispatch_to, run_command};
use git2;
use github;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str;
use webbrowser;
use Error;
use Result;

/// Parses git's configuration and extracts all aliases that do not shell out. Returns (key, value)
/// representations.
pub fn get_aliases() -> HashMap<String, String> {
    let mut rv = HashMap::new();
    let config = git2::Config::open_default().unwrap();
    let entries = config.entries(Some("alias.*")).unwrap();
    for entry_or_err in &entries {
        let entry = entry_or_err.unwrap();
        // We only need to understand aliases for git commands (like checkout, branch) and so on.
        // We will never care for stuff that shells out.
        if entry.name().unwrap().trim().starts_with('!') {
            continue;
        }
        // name is alias.<alias>, so we trim the first 6 characters.
        rv.insert(
            entry.name().unwrap()[6..].to_string(),
            entry.value().unwrap().to_string(),
        );
    }
    rv
}

/// Returns the names of all local branches.
pub fn get_all_local_branches(repo: &git2::Repository) -> Result<HashSet<String>> {
    let mut b = HashSet::new();
    for entry in repo.branches(Some(git2::BranchType::Local))? {
        let (branch, _) = entry?;
        b.insert(branch.name()?.unwrap().to_string());
    }
    Ok(b)
}

#[derive(Debug)]
struct Remote {
    url: String,
}

impl Remote {
    /// The project part of the URL, i.e. for git@github.com:SirVer/giti.git, this would be
    /// 'giti.git'.
    pub fn project(&self) -> &str {
        self.url.rsplitn(2, '/').nth(0).unwrap()
    }

    pub fn repository(&self) -> github::Repo {
        let owner_and_project = self.url.rsplitn(2, ':').nth(0).unwrap();
        let owner = owner_and_project.rsplitn(2, '/').nth(1).unwrap();
        let mut name = owner_and_project.rsplitn(2, '/').nth(0).unwrap();
        if name.ends_with(".git") {
            name = &name[..name.len() - 4];
        }
        github::Repo {
            owner: owner.to_string(),
            name: name.to_string(),
        }
    }
}

/// Returns a map from origin name to Remote.
fn get_remotes() -> Result<HashMap<String, Remote>> {
    let stdout = String::from_utf8(communicate(&["git", "remote", "-v"])?.stdout).unwrap();
    let mut result = HashMap::new();
    for line in stdout.lines() {
        if line.contains("(push)") {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it.next().unwrap();
        let origin = Remote {
            url: it.next().unwrap().to_string(),
        };
        result.insert(name.to_string(), origin);
    }
    Ok(result)
}

/// Returns the deleted or modified files in the working directory. This shells out to git
/// directly, because using `libgit2::Repository::statuses`() was very, very slow.
pub fn status() -> Result<(HashSet<PathBuf>, HashSet<PathBuf>)> {
    let mut deleted = HashSet::<PathBuf>::new();
    let mut modified = HashSet::<PathBuf>::new();

    let stdout =
        String::from_utf8(communicate(&["git", "status", "--porcelain", "-uno"])?.stdout).unwrap();
    for line in stdout.lines() {
        let entries = line.trim().splitn(2, ' ').collect::<Vec<_>>();
        match entries[0] {
            "M" => modified.insert(PathBuf::from(entries[1])),
            "D" => deleted.insert(PathBuf::from(entries[1])),
            _ => panic!("Unknow status output from git: '{}'", line),
        };
    }
    Ok((deleted, modified))
}

/// Returns an error if the working directory is dirty.
fn expect_working_directory_clean() -> Result<()> {
    let (deleted, changed) = status()?;
    if deleted.len() + changed.len() == 0 {
        return Ok(());
    }

    let mut error = String::from(
        "You cannot have pending changes for this command. Changed \
         files:\n\n",
    );
    for s in deleted.union(&changed) {
        error.push_str(&format!("  {}\n", s.to_string_lossy()));
    }
    error.push('\n');
    Err(Error::general(error))
}

/// Returns the name of the branch that is currently checked out.
pub fn get_current_branch(repo: &git2::Repository) -> String {
    let head = repo.head().unwrap();
    head.shorthand().unwrap().to_string()
}

#[derive(Debug)]
struct OriginBranch {
    remote: String,
    branch: String,
}

fn get_origin(local_branch: &str) -> Option<OriginBranch> {
    let remote = match communicate(&["git", "config", &format!("branch.{}.remote", local_branch)]) {
        Ok(out) => str::from_utf8(&out.stdout).unwrap().trim().to_string(),
        Err(_) => return None,
    };

    let branch = match communicate(&["git", "config", &format!("branch.{}.merge", local_branch)]) {
        Ok(out) => str::from_utf8(&out.stdout)
            .unwrap()
            .trim()
            .trim_left_matches("refs/heads/")
            .to_string(),
        Err(_) => return None,
    };

    Some(OriginBranch { remote, branch })
}

/// Returns the (added, deleted, modified) files between two treeishs, e.g. branch names.
pub fn get_changed_files(
    repo: &git2::Repository,
    old: &str,
    new: &str,
) -> Result<(HashSet<PathBuf>, HashSet<PathBuf>, HashSet<PathBuf>)> {
    let parent = repo.revparse_single(old)?;
    let current = repo.revparse_single(new)?;
    let merge_base = repo.find_object(repo.merge_base(parent.id(), current.id())?, None)?;

    let mut diff_options = git2::DiffOptions::new();
    diff_options
        .include_ignored(false)
        .include_untracked(false)
        .include_typechange(false)
        .ignore_filemode(true)
        .skip_binary_check(true)
        .enable_fast_untracked_dirs(true);
    let diff = repo.diff_tree_to_tree(
        merge_base.peel(git2::ObjectType::Tree)?.as_tree(),
        current.peel(git2::ObjectType::Tree)?.as_tree(),
        Some(&mut diff_options),
    )?;

    let mut added = HashSet::<PathBuf>::new();
    let mut deleted = HashSet::<PathBuf>::new();
    let mut modified = HashSet::<PathBuf>::new();
    diff.print(git2::DiffFormat::NameStatus, |_delte, _hunk, line| {
        // line is 'A\tfile/path\n'
        let path = PathBuf::from(str::from_utf8(&line.content()[2..]).unwrap().trim());
        match line.content()[0] as char {
            'A' => added.insert(path),
            'D' => deleted.insert(path),
            'M' => modified.insert(path),
            unknown => panic!("Unexpected status char: {}", unknown),
        };
        true
    })?;
    Ok((added, deleted, modified))
}

fn run_clang_format(path: &Path) -> Result<()> {
    dispatch_to(
        "clang-format",
        &[
            "-i",
            "-sort-includes",
            "-style=file",
            "-fallback-style=Google",
            &path.to_string_lossy(),
        ],
    )?;
    Ok(())
}

fn run_buildifier(path: &Path) -> Result<()> {
    dispatch_to("buildifier", &[&path.to_string_lossy()])?;
    Ok(())
}

fn run_rustfmt(path: &Path) -> Result<()> {
    dispatch_to(
        "rustup",
        &[
            "run",
            "nightly",
            "rustfmt",
            "--write-mode",
            "overwrite",
            &path.to_string_lossy(),
        ],
    )?;
    Ok(())
}

pub fn handle_fix(args: &[&str], repo: &git2::Repository) -> Result<()> {
    expect_working_directory_clean()?;

    let other_branch = if args.len() == 2 {
        &args[1]
    } else {
        "origin/master"
    };

    println!("Fixing modified files compared to {}", other_branch);
    let (added, _, modified) = get_changed_files(repo, other_branch, &get_current_branch(repo))?;

    let workdir = repo.workdir().unwrap();
    for path in added.union(&modified) {
        if path.file_name().is_none() {
            continue;
        }
        let file_name = path.file_name().unwrap().to_str().unwrap();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let full_path = workdir.join(path);

        match (file_name, ext) {
            (_, "h") | (_, "cc") | (_, "proto") => run_clang_format(&full_path)?,
            (_, "rs") => run_rustfmt(&full_path)?,
            ("BUILD", _) | (_, "BUILD") => run_buildifier(&full_path)?,
            _ => (),
        }
    }

    let changed_files = status()?.1;
    if !changed_files.is_empty() {
        println!("Fixed files:\n");
        for filename in changed_files {
            println!("  {}", filename.to_string_lossy());
        }
        println!();
        dispatch_to("git", &["commit", "-am", "Ran git fix."])?;
    }
    Ok(())
}

pub fn handle_cleanup(repo: &git2::Repository) -> Result<()> {
    let current_branch = get_current_branch(repo);
    for branch in get_all_local_branches(repo)? {
        if branch.starts_with('|') && branch != current_branch {
            run_command(&["git", "branch", "-D", &branch])?;
        }
    }
    Ok(())
}

pub fn handle_review_push(repo: &git2::Repository) -> Result<()> {
    // branch name will be user/branch_name.
    let full_branch_name = get_current_branch(repo);
    let (user, branch_name) = {
        let mut it = full_branch_name.splitn(2, '/');
        // Slice off the leading '|'
        (&it.next().unwrap()[1..], it.next().unwrap())
    };
    run_command(&[
        "git",
        "push",
        "--force",
        user,
        &format!("HEAD:{}", branch_name),
    ])?;
    Ok(())
}

pub fn handle_review(args: &[&str], repo: &git2::Repository) -> Result<()> {
    let remotes = get_remotes()?;

    let master_origin = get_origin("master").unwrap();
    let master_remote = &remotes[&master_origin.remote];
    let github_repo = master_remote.repository();

    if args.len() == 1 {
        let prs = github::find_assigned_prs(Some(&github_repo))?;
        if prs.is_empty() {
            println!(
                "No reviews assigned in {}/{}.",
                github_repo.owner, github_repo.name
            );
        } else {
            for pr in &prs {
                println!(
                    "#{} by @{}: {} ({}:{})",
                    pr.number, pr.author_login, pr.title, pr.source.repo.owner, pr.source.name
                );
            }
        }
        return Ok(());
    }

    if args.len() != 2 {
        return Err(Error::general(
            "review requires a pull request number or a user/branch_name to review.".into(),
        ));
    }

    expect_working_directory_clean()?;

    if args[1] == "push" {
        return handle_review_push(repo);
    }

    let source_branch = if let Ok(pr_number) = args[1].parse::<i32>() {
        let pr = github::get_pr(&github_repo, pr_number)?;
        pr.source
    } else {
        let (user, branch) = {
            let mut it = args[1].splitn(2, ':');
            (it.next().unwrap(), it.next().unwrap())
        };

        github::Branch {
            repo: github::Repo {
                owner: user.to_string(),
                name: github_repo.name.clone(),
            },
            name: branch.to_string(),
        }
    };

    let owner = if source_branch.repo == github_repo {
        "origin"
    } else {
        &source_branch.repo.owner
    };

    if !remotes.contains_key(owner) {
        run_command(&[
            "git",
            "remote",
            "add",
            owner,
            &format!("git@github.com:{}/{}", owner, master_remote.project()),
        ])?;
    }
    // Since the local_branch name is the remote/branch git also resolves it to the correct remote.
    run_command(&["git", "fetch", owner])?;
    let branch_to_fork = format!("remotes/{}/{}", owner, source_branch.name);
    let local_branch = format!("|{}/{}", owner, source_branch.name);

    if get_all_local_branches(repo)?.contains(&local_branch) {
        run_command(&["git", "branch", "-D", &local_branch])?;
    }

    run_command(&["git", "branch", "--track", &local_branch, &branch_to_fork])?;
    run_command(&["git", "checkout", &local_branch])?;
    Ok(())
}

pub fn handle_open_reviews(args: &[&str]) -> Result<()> {
    if args.len() != 2 {
        return Err(Error::general(
            "open_reviews requires a base url as first argument.".into(),
        ));
    }

    let prs = github::find_assigned_prs(None)?;
    for pr in prs {
        // Ignore the result.
        let _ = webbrowser::open(&format!(
            "{}{}/{}/{}",
            args[1], pr.target.repo.owner, pr.target.repo.name, pr.number
        ));
    }
    Ok(())
}

pub fn handle_repository(original_args: &[&str]) -> Result<()> {
    if original_args.is_empty() {
        return dispatch_to("git", original_args);
    }

    // Arguments that are valid without a git repository.
    match original_args[0] as &str {
        // Intercepted commands.
        "open_reviews" => return handle_open_reviews(original_args),
        _ => (),
    };

    let repo = git2::Repository::discover(".");
    if repo.is_err() {
        return dispatch_to("git", original_args);
    }
    let repo = repo.unwrap();

    match original_args[0] as &str {
        // Intercepted commands.
        "cleanup" => handle_cleanup(&repo),
        "fix" => handle_fix(original_args, &repo),
        "review" => handle_review(original_args, &repo),

        _ => dispatch_to("git", original_args),
    }
}
