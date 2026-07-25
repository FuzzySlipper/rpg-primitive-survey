use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::git_source;
use crate::model::{
    Compatibility, Diagnostic, DocumentEvidence, InventoryCounts, InventoryReceipt, Limits,
    PackInventory, ReceiptStatus, RepositoryReceipt, ResumeState, SCHEMA_VERSION, SourceIdentity,
    SystemManifest,
};
use crate::safety::{contained_join, directory_bytes, visit_files};
use crate::shape::structural_signature;

pub const DEFAULT_SOURCE_POINTERS: &[&str] = &[
    "/system/source/id",
    "/system/source/book",
    "/system/source/title",
    "/system/publication/title",
    "/source/id",
    "/source",
    "/_stats/compendiumSource",
];

pub struct InventoryOptions {
    pub study_id: String,
    pub repository: String,
    pub requested_ref: String,
    pub manifest_path: String,
    pub content_roots: Vec<String>,
    pub source_pointers: Vec<String>,
    pub source_fallbacks: BTreeMap<String, String>,
    pub limits: Limits,
    pub studies_root: PathBuf,
}

struct PackSource {
    id: String,
    path: String,
    document_type: Option<String>,
}

struct DecodeCursor {
    file: String,
    document_index: usize,
}

struct FileDiscovery {
    document_files: Vec<(String, PackSource)>,
    unsupported_by_pack: BTreeMap<String, Vec<String>>,
    missing_packs: Vec<String>,
}

pub fn run(options: &InventoryOptions) -> Result<InventoryReceipt, String> {
    let study_root = contained_join(&options.studies_root, Path::new(&options.study_id))?;
    fs::create_dir_all(&study_root)
        .map_err(|error| format!("failed to create {}: {error}", study_root.display()))?;
    let checkout = contained_join(&study_root, Path::new("checkout"))?;
    let inventory_path = contained_join(&study_root, Path::new("inventory.json"))?;

    let previous = read_previous(&inventory_path)?;
    let inspected = if let Some(receipt) = &previous {
        if receipt.repository.url != options.repository
            || receipt.repository.requested_ref != options.requested_ref
        {
            return Err(
                "existing study repository/ref differs; choose another study id".to_owned(),
            );
        }
        git_source::RemoteRefEvidence {
            commit: receipt.repository.pinned_commit.clone(),
        }
    } else {
        git_source::inspect_remote(&options.repository, &options.requested_ref)?
    };

    git_source::clone_pinned(
        &options.repository,
        &options.requested_ref,
        &inspected.commit,
        &checkout,
    )?;
    let source_pointers = if options.source_pointers.is_empty() {
        DEFAULT_SOURCE_POINTERS
            .iter()
            .map(ToString::to_string)
            .collect()
    } else {
        options.source_pointers.clone()
    };
    let repository_bytes = directory_bytes(&checkout)?;
    if repository_bytes > options.limits.max_repository_bytes {
        let receipt = size_limited_receipt(
            options,
            &source_pointers,
            &inspected.commit,
            repository_bytes,
        );
        write_json(&inventory_path, &receipt)?;
        return Ok(receipt);
    }

    let manifest_file = contained_join(&checkout, Path::new(&options.manifest_path))?;
    let manifest_value = read_json(&manifest_file)?;
    let manifest = parse_manifest(&options.manifest_path, &manifest_value);
    let pack_sources = collect_pack_sources(&manifest_value, &options.content_roots);
    if let Some(receipt) = &previous
        && (receipt.source_pointers != source_pointers
            || receipt.source_fallbacks != options.source_fallbacks)
    {
        return Err(
            "existing study source-pointer configuration differs; choose another study id"
                .to_owned(),
        );
    }

    let mut discovery = discover_pack_files(&checkout, &pack_sources)?;
    discovery
        .document_files
        .sort_by(|left, right| left.0.cmp(&right.0));

    let (mut documents, resume_cursor) = previous.map_or_else(
        || (Vec::new(), None),
        |receipt| {
            let cursor = match (
                receipt.resume.next_file.clone(),
                receipt.resume.next_document_index,
            ) {
                (Some(file), Some(document_index)) if receipt.status == ReceiptStatus::Partial => {
                    Some(DecodeCursor {
                        file,
                        document_index,
                    })
                }
                _ => None,
            };
            if cursor.is_some() {
                (receipt.documents, cursor)
            } else {
                (Vec::new(), None)
            }
        },
    );

    let mut start_reached = resume_cursor.is_none();
    let mut limit_cursor = None;
    'files: for (relative_file, pack) in &discovery.document_files {
        if !start_reached {
            start_reached = resume_cursor
                .as_ref()
                .is_some_and(|cursor| cursor.file == *relative_file);
            if !start_reached {
                continue;
            }
        }
        let values = decode_documents(&checkout.join(relative_file))?;
        let start_index = resume_cursor
            .as_ref()
            .filter(|cursor| cursor.file == *relative_file)
            .map_or(0, |cursor| cursor.document_index);
        for (index, value) in values.into_iter().enumerate().skip(start_index) {
            if documents.len() >= options.limits.max_decoded_documents {
                limit_cursor = Some(DecodeCursor {
                    file: relative_file.clone(),
                    document_index: index,
                });
                break 'files;
            }
            let pointer = format!("{relative_file}#{index}");
            documents.push(evidence_for(
                pointer,
                pack,
                &value,
                &source_pointers,
                &options.source_fallbacks,
            ));
        }
    }

    let mut packs = pack_sources
        .iter()
        .map(|pack| {
            let path = checkout.join(&pack.path);
            let bytes = directory_bytes(&path).unwrap_or(0);
            let decoded_documents = documents
                .iter()
                .filter(|document| document.pack_id == pack.id)
                .count();
            PackInventory {
                id: pack.id.clone(),
                path: pack.path.clone(),
                exists: path.exists(),
                document_type: pack.document_type.clone(),
                bytes,
                decodable_files: discovery
                    .document_files
                    .iter()
                    .filter(|(_, owner)| owner.id == pack.id)
                    .count(),
                decoded_documents,
                unsupported_files: discovery
                    .unsupported_by_pack
                    .get(&pack.id)
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    packs.sort_by(|left, right| left.id.cmp(&right.id));

    let counts = count_documents(&documents);
    let unresolved = counts
        .missing_source_documents
        .saturating_add(counts.ambiguous_source_documents);
    let unresolved_limited = unresolved > options.limits.max_unresolved_documents;
    let unsupported_files = discovery
        .unsupported_by_pack
        .values()
        .map(Vec::len)
        .sum::<usize>();
    let mut diagnostics = Vec::new();
    if limit_cursor.is_some() {
        diagnostics.push(Diagnostic {
                code: "SURVEY_DECODED_DOCUMENT_LIMIT".to_owned(),
                message: "decoded-document bound reached; rerun this study with a larger limit to continue at the pinned revision".to_owned(),
                limit: Some("maxDecodedDocuments".to_owned()),
                observed: Some(documents.len() as u64),
                maximum: Some(options.limits.max_decoded_documents as u64),
            });
    }
    if unresolved_limited {
        diagnostics.push(Diagnostic {
                code: "SURVEY_UNRESOLVED_DOCUMENT_LIMIT".to_owned(),
                message: "unclassified/ambiguous document bound exceeded; narrow the source layout or raise the explicit limit".to_owned(),
                limit: Some("maxUnresolvedDocuments".to_owned()),
                observed: Some(unresolved as u64),
                maximum: Some(options.limits.max_unresolved_documents as u64),
            });
    }
    if unsupported_files > 0 {
        diagnostics.push(Diagnostic {
            code: "SURVEY_UNSUPPORTED_PACK_LAYOUT".to_owned(),
            message: "pack files were not decoded; add a system-specific layout/decoder before claiming a complete inventory".to_owned(),
            limit: None,
            observed: Some(unsupported_files as u64),
            maximum: None,
        });
    }
    if !discovery.missing_packs.is_empty() {
        diagnostics.push(Diagnostic {
            code: "SURVEY_PACK_PATH_MISSING".to_owned(),
            message: format!(
                "manifest/content-root pack paths do not exist: {}",
                discovery.missing_packs.join(", ")
            ),
            limit: None,
            observed: Some(discovery.missing_packs.len() as u64),
            maximum: None,
        });
    }
    let status = if diagnostics.is_empty() {
        ReceiptStatus::Complete
    } else {
        ReceiptStatus::Partial
    };
    let resume = if diagnostics.is_empty() {
        ResumeState {
            resumable: false,
            next_file: None,
            next_document_index: None,
            instruction: "inventory traversal completed within configured limits".to_owned(),
        }
    } else {
        ResumeState {
            resumable: true,
            next_file: limit_cursor.as_ref().map(|cursor| cursor.file.clone()),
            next_document_index: limit_cursor.as_ref().map(|cursor| cursor.document_index),
            instruction: if limit_cursor.is_some() {
                "rerun inventory with the same study id and a larger --max-decoded-documents; also resolve any other reported diagnostics".to_owned()
            } else {
                "adjust the reported bounds or add an explicit system profile/decoder, then rerun at the same pinned revision".to_owned()
            },
        }
    };

    let receipt = InventoryReceipt {
        schema_version: SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        study_id: options.study_id.clone(),
        status,
        repository: RepositoryReceipt {
            url: options.repository.clone(),
            requested_ref: options.requested_ref.clone(),
            pinned_commit: inspected.commit,
            checkout_relative_path: "checkout".to_owned(),
            repository_bytes,
        },
        manifest,
        limits: options.limits.clone(),
        source_pointers,
        source_fallbacks: options.source_fallbacks.clone(),
        packs,
        counts,
        documents,
        diagnostics,
        resume,
    };
    write_json(&inventory_path, &receipt)?;
    Ok(receipt)
}

fn size_limited_receipt(
    options: &InventoryOptions,
    source_pointers: &[String],
    commit: &str,
    repository_bytes: u64,
) -> InventoryReceipt {
    InventoryReceipt {
        schema_version: SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        study_id: options.study_id.clone(),
        status: ReceiptStatus::Partial,
        repository: RepositoryReceipt {
            url: options.repository.clone(),
            requested_ref: options.requested_ref.clone(),
            pinned_commit: commit.to_owned(),
            checkout_relative_path: "checkout".to_owned(),
            repository_bytes,
        },
        manifest: SystemManifest {
            path: options.manifest_path.clone(),
            id: None,
            title: None,
            version: None,
            compatibility: Compatibility::default(),
        },
        limits: options.limits.clone(),
        source_pointers: source_pointers.to_vec(),
        source_fallbacks: options.source_fallbacks.clone(),
        packs: Vec::new(),
        counts: InventoryCounts::default(),
        documents: Vec::new(),
        diagnostics: vec![Diagnostic {
            code: "SURVEY_REPOSITORY_SIZE_LIMIT".to_owned(),
            message: "checked-out repository exceeds the configured storage/download safety bound"
                .to_owned(),
            limit: Some("maxRepositoryBytes".to_owned()),
            observed: Some(repository_bytes),
            maximum: Some(options.limits.max_repository_bytes),
        }],
        resume: ResumeState {
            resumable: true,
            next_file: None,
            next_document_index: None,
            instruction:
                "remove the study or rerun with an explicitly larger --max-repository-bytes"
                    .to_owned(),
        },
    }
}

fn read_previous(path: &Path) -> Result<Option<InventoryReceipt>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let value =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&value)
        .map(Some)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    fs::write(path, bytes).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn parse_manifest(path: &str, value: &Value) -> SystemManifest {
    SystemManifest {
        path: path.to_owned(),
        id: string_at(value, "/id").or_else(|| string_at(value, "/name")),
        title: string_at(value, "/title"),
        version: string_at(value, "/version"),
        compatibility: Compatibility {
            minimum: string_at(value, "/compatibility/minimum"),
            verified: string_at(value, "/compatibility/verified"),
            maximum: string_at(value, "/compatibility/maximum"),
        },
    }
}

fn collect_pack_sources(manifest: &Value, extra_roots: &[String]) -> Vec<PackSource> {
    let mut packs = Vec::new();
    if let Some(values) = manifest.get("packs").and_then(Value::as_array) {
        for (index, value) in values.iter().enumerate() {
            if let Some(path) = value.get("path").and_then(Value::as_str) {
                packs.push(PackSource {
                    id: value
                        .get("name")
                        .or_else(|| value.get("id"))
                        .and_then(Value::as_str)
                        .map_or_else(|| format!("manifest-pack-{index}"), str::to_owned),
                    path: path.to_owned(),
                    document_type: value
                        .get("type")
                        .or_else(|| value.get("documentType"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
            }
        }
    }
    if let Some(values) = manifest.get("packs").and_then(Value::as_object) {
        for (id, value) in values {
            if let Some(path) = value.get("path").and_then(Value::as_str) {
                packs.push(PackSource {
                    id: id.clone(),
                    path: path.to_owned(),
                    document_type: value
                        .get("type")
                        .or_else(|| value.get("documentType"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
            }
        }
    }
    for (index, path) in extra_roots.iter().enumerate() {
        packs.push(PackSource {
            id: format!("explicit-root-{index}"),
            path: path.clone(),
            document_type: None,
        });
    }
    packs.sort_by(|left, right| left.path.cmp(&right.path));
    packs.dedup_by(|left, right| left.path == right.path);
    packs
}

fn discover_pack_files(checkout: &Path, packs: &[PackSource]) -> Result<FileDiscovery, String> {
    let mut discovery = FileDiscovery {
        document_files: Vec::new(),
        unsupported_by_pack: BTreeMap::new(),
        missing_packs: Vec::new(),
    };
    for pack in packs {
        let root = contained_join(checkout, Path::new(&pack.path))?;
        if !root.exists() {
            discovery.missing_packs.push(pack.path.clone());
            continue;
        }
        if root.is_file() {
            classify_pack_file(checkout, &root, pack, &mut discovery)?;
            continue;
        }
        visit_files(&root, &mut |path| {
            classify_pack_file(checkout, path, pack, &mut discovery)
        })?;
    }
    Ok(discovery)
}

fn classify_pack_file(
    checkout: &Path,
    path: &Path,
    pack: &PackSource,
    discovery: &mut FileDiscovery,
) -> Result<(), String> {
    let extension = path.extension().and_then(|value| value.to_str());
    let relative = path
        .strip_prefix(checkout)
        .map_err(|error| format!("failed to make evidence pointer: {error}"))?
        .to_string_lossy()
        .replace('\\', "/");
    if matches!(extension, Some("json" | "jsonl" | "ndjson")) {
        discovery.document_files.push((
            relative,
            PackSource {
                id: pack.id.clone(),
                path: pack.path.clone(),
                document_type: pack.document_type.clone(),
            },
        ));
    } else {
        discovery
            .unsupported_by_pack
            .entry(pack.id.clone())
            .or_default()
            .push(relative);
    }
    Ok(())
}

fn decode_documents(path: &Path) -> Result<Vec<Value>, String> {
    let extension = path.extension().and_then(|value| value.to_str());
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if matches!(extension, Some("jsonl" | "ndjson")) {
        String::from_utf8(bytes)
            .map_err(|error| format!("{} is not UTF-8: {error}", path.display()))?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|error| format!("failed to decode {}: {error}", path.display()))
            })
            .collect()
    } else {
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to decode {}: {error}", path.display()))?;
        Ok(match value {
            Value::Array(values) => values,
            other => vec![other],
        })
    }
}

fn evidence_for(
    pointer: String,
    pack: &PackSource,
    value: &Value,
    source_pointers: &[String],
    source_fallbacks: &BTreeMap<String, String>,
) -> DocumentEvidence {
    let source = match classify_source(value, source_pointers) {
        SourceIdentity::Missing => {
            source_fallbacks
                .get(&pack.id)
                .map_or(SourceIdentity::Missing, |value| {
                    SourceIdentity::Classified {
                        value: value.clone(),
                        pointer: format!("fallback:pack:{}", pack.id),
                    }
                })
        }
        classified_or_ambiguous => classified_or_ambiguous,
    };
    let document_type = value
        .get("documentType")
        .or_else(|| value.get("documentName"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| pack.document_type.clone())
        .unwrap_or_else(|| "Unknown".to_owned());
    let subtype = value.get("type").and_then(Value::as_str).map(str::to_owned);
    DocumentEvidence {
        pointer,
        pack_id: pack.id.clone(),
        document_type,
        subtype,
        source,
        structural_signature: structural_signature(value),
    }
}

pub fn classify_source(value: &Value, pointers: &[String]) -> SourceIdentity {
    let mut evidence = pointers
        .iter()
        .filter_map(|pointer| {
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|source| !source.is_empty())
                .map(|source| (source.to_owned(), pointer.clone()))
        })
        .collect::<Vec<_>>();
    evidence.sort();
    evidence.dedup();
    let values = evidence
        .iter()
        .map(|(value, _)| value.clone())
        .collect::<BTreeSet<_>>();
    match values.len() {
        0 => SourceIdentity::Missing,
        1 => {
            let value = values.into_iter().next().expect("one value exists");
            let pointer = evidence
                .into_iter()
                .find_map(|(candidate, pointer)| (candidate == value).then_some(pointer))
                .expect("one pointer exists");
            SourceIdentity::Classified { value, pointer }
        }
        _ => SourceIdentity::Ambiguous {
            values: values.into_iter().collect(),
            pointers: evidence.into_iter().map(|(_, pointer)| pointer).collect(),
        },
    }
}

fn count_documents(documents: &[DocumentEvidence]) -> InventoryCounts {
    let mut counts = InventoryCounts {
        decoded_documents: documents.len(),
        ..InventoryCounts::default()
    };
    for document in documents {
        *counts
            .by_document_type
            .entry(document.document_type.clone())
            .or_default() += 1;
        if let Some(subtype) = &document.subtype {
            *counts
                .by_subtype
                .entry(document.document_type.clone())
                .or_default()
                .entry(subtype.clone())
                .or_default() += 1;
        }
        match &document.source {
            SourceIdentity::Classified { value, .. } => {
                counts.classified_documents += 1;
                *counts.by_source.entry(value.clone()).or_default() += 1;
            }
            SourceIdentity::Missing => counts.missing_source_documents += 1,
            SourceIdentity::Ambiguous { .. } => counts.ambiguous_source_documents += 1,
        }
    }
    counts
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(|candidate| match candidate {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::classify_source;
    use crate::model::SourceIdentity;

    #[test]
    fn source_identity_is_classified_missing_or_ambiguous() {
        let pointers = vec!["/system/source/id".to_owned(), "/source".to_owned()];
        assert!(matches!(
            classify_source(&json!({"system": {"source": {"id": "core"}}}), &pointers),
            SourceIdentity::Classified { value, .. } if value == "core"
        ));
        assert_eq!(
            classify_source(&json!({"name": "No Source"}), &pointers),
            SourceIdentity::Missing
        );
        assert!(matches!(
            classify_source(
                &json!({"system": {"source": {"id": "core"}}, "source": "supplement"}),
                &pointers
            ),
            SourceIdentity::Ambiguous { .. }
        ));
    }
}
