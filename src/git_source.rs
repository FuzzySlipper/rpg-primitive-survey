use std::path::Path;
use std::process::Command;

const MATERIALIZATION_RESERVE_BYTES: u64 = 1_048_576;

pub struct RemoteRefEvidence {
    pub commit: String,
}

pub enum MaterializeOutcome {
    Ready,
    LimitExceeded { observed_bytes: u64 },
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

pub fn materialize_pinned(
    repository: &str,
    expected_commit: &str,
    checkout: &Path,
    max_repository_bytes: u64,
) -> Result<MaterializeOutcome, String> {
    if checkout.exists() {
        let head = head_commit(checkout)?;
        if head != expected_commit {
            return Err(format!(
                "existing checkout is pinned to {head}, expected {expected_commit}; use a different study id"
            ));
        }
        assert_clean_checkout(checkout)?;
        let observed_bytes = crate::safety::directory_bytes(checkout)?;
        if observed_bytes > max_repository_bytes {
            remove_incomplete_checkout(checkout)?;
            return Ok(MaterializeOutcome::LimitExceeded { observed_bytes });
        }
        return Ok(MaterializeOutcome::Ready);
    }

    if max_repository_bytes <= MATERIALIZATION_RESERVE_BYTES {
        return Ok(MaterializeOutcome::LimitExceeded {
            observed_bytes: MATERIALIZATION_RESERVE_BYTES.saturating_add(1),
        });
    }
    ensure_prlimit_available()?;
    if let Some(observed_bytes) = local_tree_estimate(repository, expected_commit)?
        && observed_bytes > max_repository_bytes
    {
        return Ok(MaterializeOutcome::LimitExceeded { observed_bytes });
    }

    let parent = checkout
        .parent()
        .ok_or_else(|| "checkout path has no parent".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    run_status(
        Command::new("git").args(["init", "--quiet"]).arg(checkout),
        "initialize bounded checkout",
    )?;
    run_status(
        Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["remote", "add", "origin", repository]),
        "configure bounded checkout remote",
    )?;

    let fetch_file_cap = max_repository_bytes.saturating_sub(MATERIALIZATION_RESERVE_BYTES) / 2;
    let fetch = Command::new("prlimit")
        .arg(format!("--fsize={fetch_file_cap}"))
        .arg("--")
        .arg("git")
        .arg("-C")
        .arg(checkout)
        .args([
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "origin",
            expected_commit,
        ])
        .output()
        .map_err(|error| format!("failed to run bounded Git fetch: {error}"))?;
    if !fetch.status.success() {
        let stderr = String::from_utf8_lossy(&fetch.stderr);
        let likely_limit = stderr.contains("File size limit exceeded")
            || stderr.contains("signal 25")
            || stderr.contains("invalid index-pack output")
            || stderr.contains("early EOF");
        remove_incomplete_checkout(checkout)?;
        if likely_limit {
            return Ok(MaterializeOutcome::LimitExceeded {
                observed_bytes: max_repository_bytes.saturating_add(1),
            });
        }
        return Err(format!("bounded Git fetch failed: {}", stderr.trim()));
    }

    let actual = command_output(
        Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["rev-parse", "FETCH_HEAD"]),
        "read fetched commit",
    )?
    .trim()
    .to_owned();
    if actual != expected_commit {
        remove_incomplete_checkout(checkout)?;
        return Err(format!(
            "fetched {actual}, expected exact pin {expected_commit}; refusing mixed-revision materialization"
        ));
    }

    let git_bytes = crate::safety::directory_bytes(checkout)?;
    let (tree_bytes, entry_count) = tree_materialization_size(checkout, expected_commit)?;
    let projected_bytes = git_bytes
        .saturating_add(tree_bytes)
        .saturating_add(entry_count.saturating_mul(256))
        .saturating_add(MATERIALIZATION_RESERVE_BYTES);
    if projected_bytes > max_repository_bytes {
        remove_incomplete_checkout(checkout)?;
        return Ok(MaterializeOutcome::LimitExceeded {
            observed_bytes: projected_bytes,
        });
    }

    run_status(
        Command::new("git").arg("-C").arg(checkout).args([
            "checkout",
            "--quiet",
            "--detach",
            expected_commit,
        ]),
        "materialize pinned checkout",
    )?;
    let observed_bytes = crate::safety::directory_bytes(checkout)?;
    if observed_bytes > max_repository_bytes {
        remove_incomplete_checkout(checkout)?;
        return Ok(MaterializeOutcome::LimitExceeded { observed_bytes });
    }
    assert_clean_checkout(checkout)?;
    Ok(MaterializeOutcome::Ready)
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

pub fn assert_clean_checkout(checkout: &Path) -> Result<(), String> {
    let status = command_output(
        Command::new("git").arg("-C").arg(checkout).args([
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignored=matching",
        ]),
        "inspect checkout cleanliness",
    )?;
    if status.trim().is_empty() {
        Ok(())
    } else {
        Err(format!(
            "SURVEY_CHECKOUT_DIRTY: pinned checkout contains tracked, untracked, or ignored changes:\n{}",
            status.trim()
        ))
    }
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

pub fn is_ignored(repository_root: &Path, relative: &str) -> Result<bool, String> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["check-ignore", "--quiet", "--no-index", relative])
        .status()
        .map_err(|error| format!("failed to inspect ignore rules for {relative}: {error}"))?;
    Ok(status.success())
}

fn ensure_prlimit_available() -> Result<(), String> {
    let output = Command::new("prlimit")
        .arg("--version")
        .output()
        .map_err(|error| {
            format!(
                "SURVEY_BOUNDED_FETCH_UNAVAILABLE: prlimit is required before fetching source data: {error}"
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "SURVEY_BOUNDED_FETCH_UNAVAILABLE: prlimit failed with status {}",
            output.status
        ))
    }
}

fn local_tree_estimate(repository: &str, commit: &str) -> Result<Option<u64>, String> {
    let path = Path::new(repository);
    if !path.is_dir() {
        return Ok(None);
    }
    let (bytes, entries) = tree_materialization_size(path, commit)?;
    Ok(Some(
        bytes
            .saturating_add(entries.saturating_mul(256))
            .saturating_add(MATERIALIZATION_RESERVE_BYTES),
    ))
}

fn tree_materialization_size(repository: &Path, commit: &str) -> Result<(u64, u64), String> {
    let output = command_output(
        Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["ls-tree", "-l", "-r", commit]),
        "measure pinned Git tree",
    )?;
    let mut bytes = 0_u64;
    let mut entries = 0_u64;
    for line in output.lines() {
        let metadata = line.split_once('\t').map_or(line, |(value, _)| value);
        let size = metadata
            .split_whitespace()
            .nth(3)
            .filter(|value| *value != "-")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        bytes = bytes.saturating_add(size);
        entries = entries.saturating_add(1);
    }
    Ok((bytes, entries))
}

fn remove_incomplete_checkout(checkout: &Path) -> Result<(), String> {
    if checkout.exists() {
        std::fs::remove_dir_all(checkout).map_err(|error| {
            format!(
                "failed to remove incomplete bounded checkout {}: {error}",
                checkout.display()
            )
        })?;
    }
    Ok(())
}

fn run_status(command: &mut Command, action: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("failed to {action}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to {action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
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
