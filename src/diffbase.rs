use crate::dispatch::{dispatch_to, run_command};
use crate::error::{Error, ErrorKind, Result};
use crate::git;
use crate::github::PullRequestId;
use getopts;
use git2;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path;

#[derive(Serialize, Deserialize, Debug)]
pub struct DiffbaseJson {
    branch: String,
    diffbase: Option<String>,
    github_pr: Option<PullRequestId>,
}

#[derive(Debug, Default)]
struct DiffbaseEntry {
    parent: Option<String>,
    children: Vec<String>,
    github_pr: Option<PullRequestId>,
}

pub struct Diffbase {
    entries: HashMap<String, DiffbaseEntry>,
    json_file_path: path::PathBuf,
}

impl Diffbase {
    pub fn new(repo: &git2::Repository) -> Result<Diffbase> {
        let mut diffbase = Diffbase {
            entries: HashMap::<String, DiffbaseEntry>::new(),
            json_file_path: repo.path().join("diffbase.json"),
        };

        for branch in git::get_all_local_branch_names(repo)? {
            diffbase.entries.insert(
                branch.to_string(),
                DiffbaseEntry {
                    children: Vec::new(),
                    parent: None,
                    github_pr: None,
                },
            );
        }

        if fs::metadata(&diffbase.json_file_path).is_err() {
            return Ok(diffbase);
        }

        // JSON database is already there.
        let mut content = String::new();
        File::open(&diffbase.json_file_path)
            .and_then(|mut file: File| file.read_to_string(&mut content))?;
        let diffbase_json: Vec<DiffbaseJson> = serde_json::from_str(&content)?;

        for entry in diffbase_json {
            if !diffbase.entries.contains_key(&entry.branch) {
                println!(
                    "Branch {} no longer exists. Removing it from the diffbase map.",
                    entry.branch
                );
                continue;
            }

            diffbase.entries.get_mut(&entry.branch).unwrap().github_pr = entry.github_pr;

            let parent_name = match entry.diffbase {
                None => continue,
                Some(ref s) => s,
            };
            if !diffbase.entries.contains_key(parent_name) {
                continue;
            }

            diffbase
                .set_diffbase_quiet(&entry.branch, parent_name)
                .expect("Could not set diffbase.");
        }
        Ok(diffbase)
    }

    fn set_diffbase_quiet(&mut self, branch: &str, diffbase: &str) -> Result<()> {
        let main_branch = git::get_main_branch();
        if diffbase == main_branch {
            return Err(Error::branch_cant_be_diffbase(diffbase));
        }
        if !self.entries.contains_key(branch) {
            self.entries.insert(branch.to_string(), Default::default());
        }
        if !self.entries.contains_key(diffbase) {
            self.entries
                .insert(diffbase.to_string(), Default::default());
        }
        self.entries.get_mut(branch).unwrap().parent = Some(diffbase.to_string());
        self.entries
            .get_mut(diffbase)
            .unwrap()
            .children
            .push(branch.to_string());
        Ok(())
    }

    pub fn set_diffbase(&mut self, branch: &str, diffbase: &str) -> Result<()> {
        self.set_diffbase_quiet(branch, diffbase)?;
        println!("Setting diffbase of {} to {}.", branch, diffbase);
        Ok(())
    }

    pub fn write_to_disk(&self) -> Result<()> {
        let mut json_entries = Vec::new();
        for (key, entry) in &self.entries {
            json_entries.push(DiffbaseJson {
                branch: key.to_string(),
                diffbase: entry.parent.clone(),
                github_pr: entry.github_pr.clone(),
            });
        }
        let json_string = serde_json::to_string_pretty(&json_entries)?;

        File::create(&self.json_file_path)
            .and_then(|mut file| write!(file, "{}", &json_string))
            .map_err(Error::from)
    }

    /// Renames the branch 'current' to 'new'.
    pub fn rename(&mut self, current: &str, new: &str) {
        let entry = self.entries.remove(current).unwrap();
        self.entries.insert(new.to_string(), entry);

        for val in self.entries.values_mut() {
            if val.parent.is_some() && val.parent.as_ref().unwrap() == current {
                val.parent = Some(new.to_string());
            }

            for child in &mut val.children {
                if child == current {
                    *child = new.to_string();
                }
            }
        }
    }

    /// Returns the name of the parent branch.
    pub fn get_parent(&self, branch: &str) -> Option<&str> {
        if let Some(entry) = self.entries.get(branch) {
            if let Some(ref parent) = entry.parent {
                return Some(parent);
            }
        }
        None
    }

    /// Returns all children. Returns none if 'branch' is not in the diffbase list.
    pub fn get_children(&self, branch: &str) -> Option<Vec<&str>> {
        let entry = match self.entries.get(branch) {
            None => return None,
            Some(e) => e,
        };

        Some(entry.children.iter().map(|s| s as &str).collect())
    }

    /// Returns the ancestor of 'branch' that has no diffbase. Might be the branch itself. Returns
    /// None if 'branch' is not a valid branch name.
    pub fn get_root<'a>(&'a self, branch: &'a str) -> Option<&'a str> {
        // Make sure branch is known to us.
        self.entries.get(branch)?;

        let mut branch = branch;
        loop {
            match self.get_parent(branch) {
                None => return Some(branch),
                Some(parent) => {
                    branch = parent;
                }
            }
        }
    }

    pub fn get_github_pr(&self, branch: &str) -> Option<&PullRequestId> {
        self.entries.get(branch).and_then(|b| b.github_pr.as_ref())
    }

    pub fn set_github_pr(&mut self, branch: &str, pr: PullRequestId) {
        if !self.entries.contains_key(branch) {
            self.entries.insert(branch.to_string(), Default::default());
        }
        self.entries.get_mut(branch).unwrap().github_pr = Some(pr);
    }
}

/// Intercepts --diffbase argument and sets diffbase accordingly.
pub fn handle_merge(args: &[&str], repo: &git2::Repository, diffbase: &mut Diffbase) -> Result<()> {
    let (_, ignored_options, positional_args) = extract_option(None, &args[1..]);

    if ignored_options.is_empty() && positional_args.len() == 1 {
        // Only do something for 'g merge <branch>'.
        if let Err(err) = diffbase.set_diffbase(&git::get_current_branch(repo), positional_args[0])
        {
            if err.kind != ErrorKind::BranchCantBeDiffbase {
                return Err(err);
            }
        }
    }
    dispatch_to("git", args)
}

/// Intercepts checkout -b branch to set the diffbase on branching.
pub fn handle_checkout(
    args: &[&str],
    repo: &git2::Repository,
    diffbase: &mut Diffbase,
) -> Result<()> {
    let (new_branch_name, ignored, positional) = extract_option(Some("-b"), &args[1..]);

    if let Some(new_branch_name) = new_branch_name {
        if let Err(err) = diffbase.set_diffbase(new_branch_name, &git::get_current_branch(repo)) {
            if err.kind != ErrorKind::BranchCantBeDiffbase {
                return Err(err);
            }
        }
    }

    if ignored.is_empty() && positional.len() == 1 {
        git::checkout(repo, positional[0])?;
    } else {
        dispatch_to("git", args)?;
    }
    Ok(())
}

/// Interjects git branch -m to catch on renames.
pub fn handle_branch(
    args: &[&str],
    repo: &git2::Repository,
    diffbase: &mut Diffbase,
) -> Result<()> {
    let (new_branch_name, _, _) = extract_option(Some("-m"), &args[1..]);

    if let Some(new_branch_name) = new_branch_name {
        let current_branch = git::get_current_branch(repo);
        println!(
            "Detected branch rename: {} -> {}",
            &current_branch, new_branch_name
        );
        diffbase.rename(&current_branch, new_branch_name);
    }
    dispatch_to("git", args)
}

/// Moves the diffbase tree upwards (towards the root).
pub fn handle_up(args: &[&str], repo: &git2::Repository, diffbase: &Diffbase) -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.optflag("r", "root", "Check out root instead of parent.");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(err) => {
            let brief = format!("{}\nUsage: g up [options]", err);
            return Err(Error::general(opts.usage(&brief)));
        }
    };

    let current_branch = git::get_current_branch(repo);
    if matches.opt_present("root") {
        let root = diffbase.get_root(&current_branch).unwrap();
        git::checkout(repo, root)
    } else {
        match diffbase.get_parent(&current_branch) {
            Some(parent) => git::checkout(repo, parent),
            None => Err(Error::general(format!(
                "{} has no diffbase.",
                current_branch
            ))),
        }
    }
}

/// Moves the diffbase tree down (towards the newest branch) if there is a unique child.
pub fn handle_down(_: &[&str], repo: &git2::Repository, diffbase: &Diffbase) -> Result<()> {
    let current_branch = git::get_current_branch(repo);
    match diffbase.get_children(&current_branch) {
        Some(ref children) if children.len() == 1 => git::checkout(repo, children[0]),
        Some(ref children) if children.is_empty() => Err(Error::general(format!(
            "{} has no branches that have it as diffbase.",
            current_branch
        ))),
        Some(ref children) => Err(Error::general(format!(
            "{} has no unique branch that has it as diffbase. \
             Contenders are {}.",
            current_branch,
            children.to_vec().join(", ")
        ))),
        None => panic!("branch not in diffbase list."),
    }
}

pub fn handle_pullc(args: &[&str], repo: &git2::Repository, diffbase: &Diffbase) -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.optflag(
        "p",
        "push",
        "Also push all branches that have a upstream and are changed.",
    );
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(err) => {
            let brief = format!("{}\nUsage: g pullc [options]", err);
            return Err(Error::general(opts.usage(&brief)));
        }
    };
    let do_push = matches.opt_present("push");

    let local_branches = git::get_all_local_branches(repo)?;
    let branch_at_start = git::get_current_branch(repo);
    let root = diffbase.get_root(&branch_at_start).unwrap();

    // Merge main into the root.
    run_command(&["git", "fetch"])?;

    let has_upstream = |s| {
        if let Some(b) = local_branches.get(s) {
            return b.upstream.is_some();
        }
        false
    };

    // Sync the root branch.
    git::checkout(repo, root)?;
    if has_upstream(root) {
        run_command(&["git", "pull"])?;
    }
    if do_push && has_upstream(root) {
        run_command(&["git", "push"])?;
    }

    fn merge_parent_into_children(
        parent: &str,
        diffbase: &Diffbase,
        repo: &git2::Repository,
        local_branches: &HashMap<String, git::BranchInfo>,
        do_push: bool,
    ) -> Result<()> {
        let has_upstream = |s| {
            if let Some(b) = local_branches.get(s) {
                return b.upstream.is_some();
            }
            false
        };

        for child in diffbase.get_children(parent).unwrap() {
            git::checkout(repo, child)?;
            if has_upstream(child) {
                run_command(&["git", "pull"])?;
            }
            git::merge(parent, repo)?;
            if do_push && has_upstream(child) {
                run_command(&["git", "push"])?;
            }
            merge_parent_into_children(child, diffbase, repo, local_branches, do_push)?;
        }
        Ok(())
    }

    merge_parent_into_children(root, diffbase, repo, &local_branches, do_push)?;

    if git::get_current_branch(repo) != branch_at_start {
        git::checkout(repo, &branch_at_start)?;
    }
    Ok(())
}

fn extract_option<'a>(
    name: Option<&str>,
    args: &'a [&str],
) -> (Option<&'a str>, Vec<&'a str>, Vec<&'a str>) {
    let mut positional_args = Vec::new();
    let mut ignored_options = Vec::new();
    let mut value = None;

    let mut i = args.iter();
    while let Some(a) = i.next() {
        if let Some(name) = name {
            if a.starts_with(name) {
                value = match a.find('=') {
                    None => Some(i.next().unwrap() as &str),
                    Some(_) => Some(a.split('=').nth(1).unwrap()),
                };
                continue;
            }
        }

        if a.starts_with('-') {
            ignored_options.push(a as &str);
        } else {
            positional_args.push(a as &str);
        }
    }
    (value, ignored_options, positional_args)
}

#[cfg(test)]
mod tests {
    use super::extract_option;

    #[test]
    fn test_extract_option() {
        let args = ["foo", "-m", "blub", "--export", "flah"];
        let (value, options, positional) = extract_option(Some("-m"), &args);
        assert_eq!(value, Some("blub"));
        assert_eq!(options, ["--export"]);
        assert_eq!(positional, ["foo", "flah"]);
    }
}
