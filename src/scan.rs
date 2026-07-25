use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::git_source;
use crate::inventory::write_json;
use crate::model::{
    Diagnostic, InventoryReceipt, Limits, ReceiptStatus, RepresentativeExample, ResumeState,
    SCHEMA_VERSION, ScanReceipt, ShapeGroup, SourceAudit, SourceIdentity, SourcePartition,
    UnclassifiedPartition,
};
use crate::safety::contained_join;

pub struct ScanOptions {
    pub study_id: String,
    pub included_sources: Vec<String>,
    pub limits: Limits,
    pub studies_root: PathBuf,
}

pub fn run(options: &ScanOptions) -> Result<ScanReceipt, String> {
    if options.included_sources.is_empty() {
        return Err("scan requires at least one --include-source".to_owned());
    }
    let study_root = contained_join(&options.studies_root, Path::new(&options.study_id))?;
    let inventory_path = contained_join(&study_root, Path::new("inventory.json"))?;
    let inventory_bytes = fs::read(&inventory_path)
        .map_err(|error| format!("failed to read {}: {error}", inventory_path.display()))?;
    let inventory: InventoryReceipt = serde_json::from_slice(&inventory_bytes)
        .map_err(|error| format!("failed to decode {}: {error}", inventory_path.display()))?;
    if inventory.status != ReceiptStatus::Complete {
        return Err(
            "inventory is partial; resume inventory before claiming a scoped scan".to_owned(),
        );
    }
    let checkout = contained_join(&study_root, Path::new("checkout"))?;
    let head = git_source::head_commit(&checkout)?;
    if head != inventory.repository.pinned_commit {
        return Err(format!(
            "checkout moved from pinned commit {} to {head}; refusing mixed-revision scan",
            inventory.repository.pinned_commit
        ));
    }

    let whitelist = options
        .included_sources
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let source_audit = audit_sources(&inventory, &whitelist);
    let mut grouped = BTreeMap::<(String, String, Option<String>), Vec<_>>::new();
    for document in &inventory.documents {
        let Some(source) = document.source.classified_value() else {
            continue;
        };
        if !whitelist.contains(source) {
            continue;
        }
        grouped
            .entry((
                document.structural_signature.clone(),
                document.document_type.clone(),
                document.subtype.clone(),
            ))
            .or_default()
            .push((document, source.to_owned()));
    }

    let observed_shapes = grouped.len();
    let mut groups = Vec::new();
    for ((signature, document_type, subtype), values) in
        grouped.into_iter().take(options.limits.max_unique_shapes)
    {
        let mut sources = BTreeMap::new();
        for (_, source) in &values {
            *sources.entry(source.clone()).or_default() += 1;
        }
        let representative_examples = values
            .iter()
            .take(options.limits.max_samples_per_signature)
            .map(|(document, source)| RepresentativeExample {
                evidence_pointer: document.pointer.clone(),
                document_type: document.document_type.clone(),
                subtype: document.subtype.clone(),
                source: source.clone(),
            })
            .collect();
        groups.push(ShapeGroup {
            structural_signature: signature,
            document_type,
            subtype,
            document_count: values.len(),
            sources,
            representative_examples,
        });
    }

    let shape_limit_hit = observed_shapes > options.limits.max_unique_shapes;
    let repository_limit_hit =
        inventory.repository.repository_bytes > options.limits.max_repository_bytes;
    let unresolved = source_audit.unclassified_or_ambiguous.document_count;
    let unresolved_limit_hit = unresolved > options.limits.max_unresolved_documents;
    let mut diagnostics = Vec::new();
    if shape_limit_hit {
        diagnostics.push(Diagnostic {
            code: "SURVEY_UNIQUE_SHAPE_LIMIT".to_owned(),
            message: "unique structural-shape bound reached; report is partial".to_owned(),
            limit: Some("maxUniqueShapes".to_owned()),
            observed: Some(observed_shapes as u64),
            maximum: Some(options.limits.max_unique_shapes as u64),
        });
    }
    if repository_limit_hit {
        diagnostics.push(Diagnostic {
            code: "SURVEY_REPOSITORY_SIZE_LIMIT".to_owned(),
            message: "pinned checkout exceeds this scan's repository-size bound".to_owned(),
            limit: Some("maxRepositoryBytes".to_owned()),
            observed: Some(inventory.repository.repository_bytes),
            maximum: Some(options.limits.max_repository_bytes),
        });
    }
    if unresolved_limit_hit {
        diagnostics.push(Diagnostic {
            code: "SURVEY_UNRESOLVED_DOCUMENT_LIMIT".to_owned(),
            message: "unclassified/ambiguous evidence exceeds this scan's bound".to_owned(),
            limit: Some("maxUnresolvedDocuments".to_owned()),
            observed: Some(unresolved as u64),
            maximum: Some(options.limits.max_unresolved_documents as u64),
        });
    }
    let partial = !diagnostics.is_empty();
    let status = if partial {
        ReceiptStatus::Partial
    } else {
        ReceiptStatus::Complete
    };
    let resume = if partial {
        ResumeState {
            resumable: true,
            next_file: None,
            next_document_index: shape_limit_hit.then_some(groups.len()),
            instruction:
                "rerun scan at the same pinned revision after raising the reported bounds or refining provenance"
                    .to_owned(),
        }
    } else {
        ResumeState {
            resumable: false,
            next_file: None,
            next_document_index: None,
            instruction: "scoped structural scan completed within configured limits".to_owned(),
        }
    };

    let receipt = ScanReceipt {
        schema_version: SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        study_id: options.study_id.clone(),
        status,
        repository: inventory.repository.clone(),
        system: inventory.manifest.clone(),
        included_source_whitelist: whitelist.into_iter().collect(),
        limits: options.limits.clone(),
        source_audit,
        shape_groups: groups,
        diagnostics,
        resume,
        interpretation_contract: json!({
            "kind": "primitive-study-worksheet",
            "not": [
                "Foundry importer",
                "content converter",
                "ASHA IR generator",
                "ruleset implementation"
            ],
            "candidateLimit": options.limits.max_candidate_primitives,
            "evidenceStrength": ["single-witness", "repeated-shape", "cross-source", "unresolved"],
            "completionMeaning": "configured repository surface traversed; semantic completeness is not claimed"
        }),
    };

    write_json(
        &contained_join(&study_root, Path::new("scan.json"))?,
        &receipt,
    )?;
    write_json(
        &contained_join(&study_root, Path::new("source-audit.json"))?,
        &receipt.source_audit,
    )?;
    write_worksheets(&study_root, &receipt)?;
    Ok(receipt)
}

fn audit_sources(inventory: &InventoryReceipt, whitelist: &BTreeSet<String>) -> SourceAudit {
    let mut included = SourcePartition::default();
    let mut excluded = SourcePartition::default();
    let mut unresolved = UnclassifiedPartition::default();
    for document in &inventory.documents {
        match &document.source {
            SourceIdentity::Classified { value, .. } if whitelist.contains(value) => {
                included.document_count += 1;
                *included.sources.entry(value.clone()).or_default() += 1;
            }
            SourceIdentity::Classified { value, .. } => {
                excluded.document_count += 1;
                *excluded.sources.entry(value.clone()).or_default() += 1;
            }
            SourceIdentity::Missing => {
                unresolved.missing += 1;
                unresolved.document_count += 1;
                unresolved.pointers.push(document.pointer.clone());
            }
            SourceIdentity::Ambiguous { .. } => {
                unresolved.ambiguous += 1;
                unresolved.document_count += 1;
                unresolved.pointers.push(document.pointer.clone());
            }
        }
    }
    SourceAudit {
        included,
        excluded_by_whitelist: excluded,
        unclassified_or_ambiguous: unresolved,
    }
}

fn write_worksheets(study_root: &Path, receipt: &ScanReceipt) -> Result<(), String> {
    let status = match receipt.status {
        ReceiptStatus::Complete => "complete within configured traversal limits",
        ReceiptStatus::Partial => "PARTIAL — do not claim study completeness",
    };
    let summary = format!(
        "# Primitive study: {}\n\nStatus: **{}**\n\nPinned source: `{}` at `{}` (`{}`)\n\nSystem manifest: `{}` version `{}`\n\nIncluded sources: {}\n\nIncluded documents: {}\nExcluded-by-whitelist documents: {}\nUnclassified or ambiguous documents: {}\nUnique structural groups retained: {}\n\nThis is a structural evidence worksheet, not an importer, ASHA implementation, or claim of semantic completeness.\n",
        receipt.study_id,
        status,
        receipt.repository.url,
        receipt.repository.requested_ref,
        receipt.repository.pinned_commit,
        receipt.system.path,
        receipt.system.version.as_deref().unwrap_or("<unknown>"),
        receipt.included_source_whitelist.join(", "),
        receipt.source_audit.included.document_count,
        receipt.source_audit.excluded_by_whitelist.document_count,
        receipt
            .source_audit
            .unclassified_or_ambiguous
            .document_count,
        receipt.shape_groups.len()
    );
    write_text(study_root, "study-summary.md", &summary)?;

    let candidates = format!(
        "# Candidate primitives\n\nCandidate limit: {}.\n\nFor each candidate, record:\n\n- stable local candidate ID;\n- concise behavior or data requirement;\n- evidence strength: single-witness, repeated-shape, cross-source, or unresolved;\n- supporting structural signatures and source pointers from `scan.json`;\n- boundary and meaningful counterexample;\n- whether it is generic, system-profile-specific, or content-specific.\n\nNo candidates are generated automatically. A survey agent must interpret evidence without copying source expression.\n",
        receipt.limits.max_candidate_primitives
    );
    write_text(study_root, "primitive-candidates.md", &candidates)?;

    let mut examples = String::from(
        "# Representative structural examples\n\nThese pointers identify local evidence; they do not reproduce source content.\n\n",
    );
    for group in &receipt.shape_groups {
        write!(
            examples,
            "## `{}`\n\nDocument: `{}` / `{}`; witnesses: {}.\n\n",
            group.structural_signature,
            group.document_type,
            group.subtype.as_deref().unwrap_or("<none>"),
            group.document_count
        )
        .expect("writing to a String cannot fail");
        for example in &group.representative_examples {
            writeln!(
                examples,
                "- `{}` (source `{}`)\n",
                example.evidence_pointer, example.source
            )
            .expect("writing to a String cannot fail");
        }
        examples.push_str("\nBoundary to investigate: _unresolved_\n\n");
    }
    write_text(study_root, "representative-examples.md", &examples)?;

    write_text(
        study_root,
        "asha-coverage.md",
        "# ASHA coverage assessment\n\nFor every interpreted candidate, record one of: supported, composable from existing primitives, missing primitive, content concern, or unresolved. Cite an exact ASHA surface and local evidence pointer. This worksheet does not authorize implementation.\n",
    )?;
    write_text(
        study_root,
        "open-questions.md",
        "# Open questions and source pointers\n\nRecord ambiguity, competing interpretations, missing evidence, limit effects, and the smallest additional source witness that could resolve each question.\n",
    )
}

fn write_text(study_root: &Path, name: &str, content: &str) -> Result<(), String> {
    let path = contained_join(study_root, Path::new(name))?;
    fs::write(&path, content)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}
