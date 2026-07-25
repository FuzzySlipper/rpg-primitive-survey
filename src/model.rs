use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub max_repository_bytes: u64,
    pub max_decoded_documents: usize,
    pub max_samples_per_signature: usize,
    pub max_unique_shapes: usize,
    pub max_candidate_primitives: usize,
    pub max_unresolved_documents: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_repository_bytes: 2_000_000_000,
            max_decoded_documents: 100_000,
            max_samples_per_signature: 3,
            max_unique_shapes: 5_000,
            max_candidate_primitives: 1_000,
            max_unresolved_documents: 10_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub limit: Option<String>,
    pub observed: Option<u64>,
    pub maximum: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeState {
    pub resumable: bool,
    pub next_file: Option<String>,
    pub next_document_index: Option<usize>,
    pub instruction: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptStatus {
    Complete,
    Partial,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryReceipt {
    pub url: String,
    pub requested_ref: String,
    pub pinned_commit: String,
    pub checkout_relative_path: String,
    pub repository_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Compatibility {
    pub minimum: Option<String>,
    pub verified: Option<String>,
    pub maximum: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemManifest {
    pub path: String,
    pub id: Option<String>,
    pub title: Option<String>,
    pub version: Option<String>,
    pub compatibility: Compatibility,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInventory {
    pub id: String,
    pub path: String,
    pub exists: bool,
    pub document_type: Option<String>,
    pub bytes: u64,
    pub decodable_files: usize,
    pub decoded_documents: usize,
    pub unsupported_files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceIdentity {
    Classified {
        value: String,
        pointer: String,
    },
    Missing,
    Ambiguous {
        values: Vec<String>,
        pointers: Vec<String>,
    },
}

impl SourceIdentity {
    pub fn classified_value(&self) -> Option<&str> {
        match self {
            Self::Classified { value, .. } => Some(value),
            Self::Missing | Self::Ambiguous { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEvidence {
    pub pointer: String,
    pub pack_id: String,
    pub document_type: String,
    pub subtype: Option<String>,
    pub source: SourceIdentity,
    pub structural_signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCounts {
    pub decoded_documents: usize,
    pub classified_documents: usize,
    pub missing_source_documents: usize,
    pub ambiguous_source_documents: usize,
    pub by_document_type: BTreeMap<String, usize>,
    pub by_subtype: BTreeMap<String, BTreeMap<String, usize>>,
    pub by_source: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryReceipt {
    pub schema_version: u32,
    pub tool_version: String,
    pub study_id: String,
    pub status: ReceiptStatus,
    pub repository: RepositoryReceipt,
    pub manifest: SystemManifest,
    pub limits: Limits,
    pub source_pointers: Vec<String>,
    pub source_fallbacks: BTreeMap<String, String>,
    pub packs: Vec<PackInventory>,
    pub counts: InventoryCounts,
    pub documents: Vec<DocumentEvidence>,
    pub diagnostics: Vec<Diagnostic>,
    pub resume: ResumeState,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourcePartition {
    pub sources: BTreeMap<String, usize>,
    pub document_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedPartition {
    pub missing: usize,
    pub ambiguous: usize,
    pub document_count: usize,
    pub pointers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAudit {
    pub included: SourcePartition,
    pub excluded_by_whitelist: SourcePartition,
    pub unclassified_or_ambiguous: UnclassifiedPartition,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepresentativeExample {
    pub evidence_pointer: String,
    pub document_type: String,
    pub subtype: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeGroup {
    pub structural_signature: String,
    pub document_type: String,
    pub subtype: Option<String>,
    pub document_count: usize,
    pub sources: BTreeMap<String, usize>,
    pub representative_examples: Vec<RepresentativeExample>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReceipt {
    pub schema_version: u32,
    pub tool_version: String,
    pub study_id: String,
    pub status: ReceiptStatus,
    pub repository: RepositoryReceipt,
    pub system: SystemManifest,
    pub included_source_whitelist: Vec<String>,
    pub limits: Limits,
    pub source_audit: SourceAudit,
    pub shape_groups: Vec<ShapeGroup>,
    pub diagnostics: Vec<Diagnostic>,
    pub resume: ResumeState,
    pub interpretation_contract: Value,
}
