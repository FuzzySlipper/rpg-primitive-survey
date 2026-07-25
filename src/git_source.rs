use std::path::Path;
use std::process::Command;

pub struct RemoteRefEvidence {
    pub commit: String,
}

pub fn inspect_remote(repository: &str, requested_ref: &str) -> Result<RemoteRefEvidence, String> {
    let requested = command_output(
        Command::new("git").args([
            "ls-remote",
            repository,
            &format!("refs/heads/{requested_ref}"),
        ]),
        "inspect requested Git ref",
    )?;
    let commit = requested
        .split_whitespace()
        .next()
        .filter(|_| !requested.trim().is_empty())
        .map(str::to_owned);

    let symref = command_output(
        Command::new("git").args(["ls-remote", "--symref", repository, "HEAD"]),
        "inspect remote default branch",
    )?;
    let detected_default = symref.lines().find_map(|line| {
        line.strip_prefix("ref: refs/heads/")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_owned)
    });

    let heads = command_output(
        Command::new("git").args(["ls-remote", "--heads", repository]),
        "enumerate remote heads",
    )?;
    let mut available_heads = heads
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|value| value.strip_prefix("refs/heads/"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    available_heads.sort();

    let Some(commit) = commit else {
        return Err(format!(
            "SURVEY_REF_UNAVAILABLE: requested ref {requested_ref:?} does not exist; detected default: {}; available heads: {}. Specify --ref explicitly; no branch was substituted",
            detected_default.as_deref().unwrap_or("<unknown>"),
            if available_heads.is_empty() {
                "<none>".to_owned()
            } else {
                available_heads.join(", ")
            }
        ));
    };

    Ok(RemoteRefEvidence { commit })
}

pub fn clone_pinned(
    repository: &str,
    requested_ref: &str,
    expected_commit: &str,
    checkout: &Path,
) -> Result<(), String> {
    if checkout.exists() {
        let head = head_commit(checkout)?;
        if head != expected_commit {
            return Err(format!(
                "existing checkout is pinned to {head}, expected {expected_commit}; use a different study id"
            ));
        }
        return Ok(());
    }

    let parent = checkout
        .parent()
        .ok_or_else(|| "checkout path has no parent".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let status = Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--no-tags",
            "--single-branch",
            "--branch",
            requested_ref,
            repository,
        ])
        .arg(checkout)
        .status()
        .map_err(|error| format!("failed to run git clone: {error}"))?;
    if !status.success() {
        return Err(format!("git clone failed with status {status}"));
    }

    let actual = head_commit(checkout)?;
    if actual != expected_commit {
        return Err(format!(
            "remote moved during clone: inspected {expected_commit}, cloned {actual}; retry to establish an unambiguous pin"
        ));
    }
    Ok(())
}

pub fn head_commit(checkout: &Path) -> Result<String, String> {
    command_output(
        Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["rev-parse", "HEAD"]),
        "read checkout commit",
    )
    .map(|output| output.trim().to_owned())
}

pub fn tracked_files(repository_root: &Path) -> Result<Vec<String>, String> {
    let output = command_output(
        Command::new("git")
            .arg("-C")
            .arg(repository_root)
            .args(["ls-files"]),
        "list tracked files",
    )?;
    Ok(output.lines().map(str::to_owned).collect())
}

fn command_output(command: &mut Command, action: &str) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("failed to {action}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to {action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{action} returned non-UTF-8 output: {error}"))
}
