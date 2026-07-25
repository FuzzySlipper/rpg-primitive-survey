use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

#[test]
fn pinned_inventory_scan_partition_dedupe_and_resume() {
    let fixture = TempDir::new().expect("fixture tempdir");
    copy_tree(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic"),
        fixture.path(),
    );
    initialize_git(fixture.path(), "main");
    let pinned = git_output(fixture.path(), &["rev-parse", "HEAD"]);
    let work = TempDir::new().expect("work tempdir");

    let partial = run(&[
        "inventory",
        "--study",
        "synthetic",
        "--repository",
        fixture.path().to_str().expect("fixture path"),
        "--work-root",
        work.path().to_str().expect("work path"),
        "--max-decoded-documents",
        "2",
    ]);
    assert!(partial.status.success(), "{}", stderr(&partial));
    let inventory_path = work.path().join("studies/synthetic/inventory.json");
    let partial_inventory = read_json(&inventory_path);
    assert_eq!(partial_inventory["status"], "partial");
    assert_eq!(
        partial_inventory["diagnostics"][0]["code"],
        "SURVEY_DECODED_DOCUMENT_LIMIT"
    );
    assert_eq!(partial_inventory["resume"]["resumable"], true);
    assert_eq!(partial_inventory["repository"]["pinnedCommit"], pinned);

    fs::write(
        fixture.path().join("post-inventory.json"),
        "{\"not\":\"part of pin\"}",
    )
    .expect("write later source file");
    git(fixture.path(), &["add", "."]);
    git(
        fixture.path(),
        &["commit", "-m", "move source after inventory"],
    );

    let resumed = run(&[
        "inventory",
        "--study",
        "synthetic",
        "--repository",
        fixture.path().to_str().expect("fixture path"),
        "--work-root",
        work.path().to_str().expect("work path"),
        "--max-decoded-documents",
        "20",
    ]);
    assert!(resumed.status.success(), "{}", stderr(&resumed));
    let inventory = read_json(&inventory_path);
    assert_eq!(inventory["status"], "complete");
    assert_eq!(inventory["repository"]["pinnedCommit"], pinned);
    assert_eq!(inventory["counts"]["decodedDocuments"], 5);
    assert_eq!(inventory["counts"]["missingSourceDocuments"], 1);
    assert_eq!(inventory["counts"]["ambiguousSourceDocuments"], 1);
    assert_eq!(inventory["counts"]["bySubtype"]["Item"]["action"], 3);

    let scan = run(&[
        "scan",
        "--study",
        "synthetic",
        "--include-source",
        "core",
        "--work-root",
        work.path().to_str().expect("work path"),
    ]);
    assert!(scan.status.success(), "{}", stderr(&scan));
    let scan_value = read_json(&work.path().join("studies/synthetic/scan.json"));
    assert_eq!(scan_value["status"], "complete");
    assert_eq!(scan_value["sourceAudit"]["included"]["documentCount"], 2);
    assert_eq!(
        scan_value["sourceAudit"]["excludedByWhitelist"]["documentCount"],
        1
    );
    assert_eq!(
        scan_value["sourceAudit"]["unclassifiedOrAmbiguous"]["documentCount"],
        2
    );
    assert_eq!(scan_value["shapeGroups"].as_array().unwrap().len(), 1);
    assert_eq!(scan_value["shapeGroups"][0]["documentCount"], 2);
    assert_eq!(
        scan_value["shapeGroups"][0]["representativeExamples"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let study_files = list_relative(work.path());
    assert!(study_files.iter().all(|path| path.starts_with("studies/")));
    for expected in [
        "source-audit.json",
        "study-summary.md",
        "primitive-candidates.md",
        "representative-examples.md",
        "asha-coverage.md",
        "open-questions.md",
    ] {
        assert!(
            work.path()
                .join("studies/synthetic")
                .join(expected)
                .is_file()
        );
    }
}

#[test]
fn missing_main_fails_without_silent_default_branch_substitution() {
    let fixture = TempDir::new().expect("fixture tempdir");
    copy_tree(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic"),
        fixture.path(),
    );
    initialize_git(fixture.path(), "trunk");
    let work = TempDir::new().expect("work tempdir");
    let output = run(&[
        "inventory",
        "--study",
        "wrong-default",
        "--repository",
        fixture.path().to_str().expect("fixture path"),
        "--work-root",
        work.path().to_str().expect("work path"),
    ]);
    assert!(!output.status.success());
    let error = stderr(&output);
    assert!(error.contains("SURVEY_REF_UNAVAILABLE"), "{error}");
    assert!(error.contains("detected default: trunk"), "{error}");
    assert!(!work.path().join("studies/wrong-default/checkout").exists());
}

#[test]
fn unique_shape_limit_is_an_explicit_partial_scan() {
    let fixture = TempDir::new().expect("fixture tempdir");
    copy_tree(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic"),
        fixture.path(),
    );
    initialize_git(fixture.path(), "main");
    let work = TempDir::new().expect("work tempdir");
    assert!(
        run(&[
            "inventory",
            "--study",
            "shape-limit",
            "--repository",
            fixture.path().to_str().expect("fixture path"),
            "--work-root",
            work.path().to_str().expect("work path"),
        ])
        .status
        .success()
    );
    let output = run(&[
        "scan",
        "--study",
        "shape-limit",
        "--include-source",
        "core",
        "--include-source",
        "supplement",
        "--work-root",
        work.path().to_str().expect("work path"),
        "--max-unique-shapes",
        "1",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scan = read_json(&work.path().join("studies/shape-limit/scan.json"));
    assert_eq!(scan["status"], "partial");
    assert_eq!(scan["diagnostics"][0]["code"], "SURVEY_UNIQUE_SHAPE_LIMIT");
    assert_eq!(scan["resume"]["resumable"], true);
}

#[test]
fn pack_fallback_classifies_missing_but_not_ambiguous_provenance() {
    let fixture = TempDir::new().expect("fixture tempdir");
    copy_tree(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic"),
        fixture.path(),
    );
    initialize_git(fixture.path(), "main");
    let work = TempDir::new().expect("work tempdir");
    let output = run(&[
        "inventory",
        "--study",
        "fallback",
        "--repository",
        fixture.path().to_str().expect("fixture path"),
        "--work-root",
        work.path().to_str().expect("work path"),
        "--source-fallback",
        "synthetic-actions=core",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let inventory = read_json(&work.path().join("studies/fallback/inventory.json"));
    assert_eq!(inventory["status"], "complete");
    assert_eq!(inventory["counts"]["classifiedDocuments"], 4);
    assert_eq!(inventory["counts"]["missingSourceDocuments"], 0);
    assert_eq!(inventory["counts"]["ambiguousSourceDocuments"], 1);
    assert!(
        inventory["documents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|document| {
                document["source"]["classified"]["pointer"] == "fallback:pack:synthetic-actions"
            })
    );
}

#[test]
fn unsupported_pack_layout_cannot_claim_complete_inventory() {
    let fixture = TempDir::new().expect("fixture tempdir");
    copy_tree(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic"),
        fixture.path(),
    );
    let manifest_path = fixture.path().join("system.json");
    let mut manifest = read_json(&manifest_path);
    manifest["packs"][0]["path"] = Value::String("packs".to_owned());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("encode manifest"),
    )
    .expect("write manifest");
    fs::write(fixture.path().join("packs/CURRENT"), "MANIFEST-000001\n")
        .expect("write unsupported pack marker");
    initialize_git(fixture.path(), "main");
    let work = TempDir::new().expect("work tempdir");
    let output = run(&[
        "inventory",
        "--study",
        "unsupported",
        "--repository",
        fixture.path().to_str().expect("fixture path"),
        "--work-root",
        work.path().to_str().expect("work path"),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let inventory = read_json(&work.path().join("studies/unsupported/inventory.json"));
    assert_eq!(inventory["status"], "partial");
    assert!(
        inventory["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "SURVEY_UNSUPPORTED_PACK_LAYOUT")
    );
    assert_eq!(
        inventory["packs"][0]["unsupportedFiles"][0],
        "packs/CURRENT"
    );
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rpg-primitive-survey"))
        .args(arguments)
        .output()
        .expect("run survey")
}

fn initialize_git(path: &Path, branch: &str) {
    git(path, &["init", "-b", branch]);
    git(path, &["config", "user.name", "Synthetic Test"]);
    git(path, &["config", "user.email", "synthetic@example.invalid"]);
    git(path, &["add", "."]);
    git(path, &["commit", "-m", "synthetic fixture"]);
}

fn git(path: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(output.status.success(), "{}", stderr(&output));
}

fn git_output(path: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(output.status.success(), "{}", stderr(&output));
    String::from_utf8(output.stdout)
        .expect("git stdout")
        .trim()
        .to_owned()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read JSON")).expect("decode JSON")
}

fn copy_tree(source: impl AsRef<Path>, destination: &Path) {
    fs::create_dir_all(destination).expect("create destination");
    for entry in fs::read_dir(source).expect("read fixture") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_tree(entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn list_relative(root: &Path) -> Vec<String> {
    fn walk(root: &Path, path: &Path, output: &mut Vec<String>) {
        for entry in fs::read_dir(path).expect("read work root") {
            let entry = entry.expect("work entry");
            let entry_path = entry.path();
            if entry.file_type().expect("work type").is_dir() {
                walk(root, &entry_path, output);
            } else {
                output.push(
                    entry_path
                        .strip_prefix(root)
                        .expect("relative work path")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut output = Vec::new();
    walk(root, root, &mut output);
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
