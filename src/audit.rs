use std::path::Path;

use crate::git_source;

const FORBIDDEN_EXTENSIONS: &[&str] = &[
    "db", "ldb", "sqlite", "sqlite3", "pack", "zip", "tar", "gz", "7z", "pdf", "png", "jpg",
    "jpeg", "gif", "webp", "mp3", "ogg", "wav", "mp4", "webm",
];

pub fn run(repository_root: &Path, max_tracked_bytes: u64) -> Result<(), String> {
    let tracked = git_source::tracked_files(repository_root)?;
    let mut violations = Vec::new();
    for probe in [".work/survey-probe", ".survey-work/survey-probe"] {
        if !git_source::is_ignored(repository_root, probe)? {
            violations.push(format!("{probe}: required local work root is not ignored"));
        }
    }
    for relative in tracked {
        if relative.starts_with(".work/")
            || relative.starts_with(".survey-work/")
            || relative.contains("/checkout/")
            || relative.contains("/studies/")
        {
            violations.push(format!("{relative}: generated survey/work-root path"));
            continue;
        }
        let path = repository_root.join(&relative);
        let extension = path.extension().and_then(|value| value.to_str());
        if extension.is_some_and(|value| FORBIDDEN_EXTENSIONS.contains(&value)) {
            violations.push(format!("{relative}: forbidden data/binary extension"));
            continue;
        }
        let size = std::fs::metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
            .len();
        if size > max_tracked_bytes {
            violations.push(format!(
                "{relative}: {size} bytes exceeds tracked-file limit {max_tracked_bytes}"
            ));
        }
    }
    if violations.is_empty() {
        println!("tracked-file audit passed");
        Ok(())
    } else {
        Err(format!(
            "tracked-file audit rejected:\n{}",
            violations.join("\n")
        ))
    }
}
