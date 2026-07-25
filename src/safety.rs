use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn validate_study_id(value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "study id {value:?} must contain only ASCII letters, digits, '-' or '_'"
        ))
    }
}

pub fn prepare_work_root(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path == Path::new("/") {
        return Err("work root must be a dedicated directory, not '/'".to_owned());
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.as_deref() == Some(path) {
        return Err("work root must not be the home directory".to_owned());
    }

    fs::create_dir_all(path)
        .map_err(|error| format!("failed to create work root {}: {error}", path.display()))?;
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("failed to resolve work root {}: {error}", path.display()))?;
    if resolved.join(".git").exists() && resolved.join("Cargo.toml").exists() {
        return Err("work root must not be a source repository root".to_owned());
    }
    Ok(resolved)
}

pub fn contained_join(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("path {} escapes its work root", relative.display()));
    }
    let joined = root.join(relative);
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        if cursor.exists()
            && fs::symlink_metadata(&cursor)
                .map_err(|error| format!("failed to inspect {}: {error}", cursor.display()))?
                .file_type()
                .is_symlink()
        {
            return Err(format!(
                "path {} crosses symlink {}, refusing work-root escape",
                relative.display(),
                cursor.display()
            ));
        }
    }
    Ok(joined)
}

pub fn directory_bytes(path: &Path) -> Result<u64, String> {
    if path.is_file() {
        return fs::metadata(path)
            .map(|metadata| metadata.len())
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()));
    }
    let mut total = 0_u64;
    visit_files(path, &mut |file| {
        total = total.saturating_add(
            fs::symlink_metadata(file)
                .map_err(|error| format!("failed to inspect {}: {error}", file.display()))?
                .len(),
        );
        Ok(())
    })?;
    Ok(total)
}

pub fn visit_files(
    root: &Path,
    visitor: &mut impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(root)
        .map_err(|error| format!("failed to read {}: {error}", root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", root.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            visit_files(&path, visitor)?;
        } else if metadata.is_file() {
            visitor(&path)?;
        }
    }
    Ok(())
}
