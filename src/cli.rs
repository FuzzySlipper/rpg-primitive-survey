use std::path::{Path, PathBuf};

use crate::audit;
use crate::inventory::{self, InventoryOptions};
use crate::model::{Limits, ReceiptStatus};
use crate::safety::{prepare_work_root, validate_study_id};
use crate::scan::{self, ScanOptions};

pub fn run(arguments: &[String]) -> Result<(), String> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage());
    };
    let options = ParsedOptions::parse(&arguments[1..])?;
    match command {
        "inventory" => inventory_command(&options),
        "scan" => scan_command(&options),
        "tracked-audit" => audit_command(&options),
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown command {other:?}\n\n{}", usage())),
    }
}

fn inventory_command(options: &ParsedOptions) -> Result<(), String> {
    options.reject_unknown(&[
        "study",
        "repository",
        "ref",
        "manifest",
        "profile",
        "content-root",
        "source-pointer",
        "source-fallback",
        "work-root",
        "max-repository-bytes",
        "max-decoded-documents",
        "max-samples-per-signature",
        "max-unique-shapes",
        "max-candidate-primitives",
        "max-unresolved-documents",
    ])?;
    let study_id = options.required_one("study")?;
    validate_study_id(&study_id)?;
    let repository = options.required_one("repository")?;
    let requested_ref = options.one("ref")?.unwrap_or_else(|| "main".to_owned());
    let work_root = prepare_work_root(&PathBuf::from(
        options
            .one("work-root")?
            .unwrap_or_else(|| ".work".to_owned()),
    ))?;
    let limits = options.limits()?;
    let profile = parse_profile(options.one("profile")?)?;
    let receipt = inventory::run(&InventoryOptions {
        study_id,
        profile,
        repository,
        requested_ref,
        manifest_path: options
            .one("manifest")?
            .unwrap_or_else(|| "system.json".to_owned()),
        content_roots: options.many("content-root"),
        source_pointers: options.many("source-pointer"),
        source_fallbacks: options.key_values("source-fallback")?,
        limits,
        studies_root: work_root.join("studies"),
    })?;
    println!(
        "inventory {} at {}: {:?} ({} documents)",
        receipt.study_id,
        receipt.repository.pinned_commit,
        receipt.status,
        receipt.documents.len()
    );
    Ok(())
}

fn scan_command(options: &ParsedOptions) -> Result<(), String> {
    options.reject_unknown(&[
        "study",
        "include-source",
        "work-root",
        "max-repository-bytes",
        "max-decoded-documents",
        "max-samples-per-signature",
        "max-unique-shapes",
        "max-candidate-primitives",
        "max-unresolved-documents",
    ])?;
    let study_id = options.required_one("study")?;
    validate_study_id(&study_id)?;
    let work_root = prepare_work_root(&PathBuf::from(
        options
            .one("work-root")?
            .unwrap_or_else(|| ".work".to_owned()),
    ))?;
    let receipt = scan::run(&ScanOptions {
        study_id,
        included_sources: options.many("include-source"),
        limits: options.limits()?,
        studies_root: work_root.join("studies"),
    })?;
    println!(
        "scan at {}: {:?} ({} structural groups)",
        receipt.repository.pinned_commit,
        receipt.status,
        receipt.shape_groups.len()
    );
    if receipt.status == ReceiptStatus::Partial {
        println!("partial report: {}", receipt.resume.instruction);
    }
    Ok(())
}

fn audit_command(options: &ParsedOptions) -> Result<(), String> {
    options.reject_unknown(&["repository-root", "max-tracked-bytes"])?;
    let root = options
        .one("repository-root")?
        .unwrap_or_else(|| ".".to_owned());
    let maximum = parse_u64(
        "max-tracked-bytes",
        options.one("max-tracked-bytes")?,
        1_000_000,
    )?;
    audit::run(Path::new(&root), maximum)
}

#[derive(Default)]
struct ParsedOptions {
    values: Vec<(String, String)>,
}

impl ParsedOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut index = 0;
        while index < arguments.len() {
            let option = arguments[index]
                .strip_prefix("--")
                .ok_or_else(|| format!("expected --option, found {:?}", arguments[index]))?;
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("missing value for --{option}"))?;
            if value.starts_with("--") {
                return Err(format!("missing value for --{option}"));
            }
            parsed.values.push((option.to_owned(), value.clone()));
            index += 2;
        }
        Ok(parsed)
    }

    fn one(&self, name: &str) -> Result<Option<String>, String> {
        let values = self.many(name);
        match values.as_slice() {
            [] => Ok(None),
            [value] => Ok(Some(value.clone())),
            _ => Err(format!("--{name} may be specified only once")),
        }
    }

    fn required_one(&self, name: &str) -> Result<String, String> {
        self.one(name)?
            .ok_or_else(|| format!("missing required --{name}"))
    }

    fn many(&self, name: &str) -> Vec<String> {
        self.values
            .iter()
            .filter(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.clone())
            .collect()
    }

    fn limits(&self) -> Result<Limits, String> {
        let defaults = Limits::default();
        Ok(Limits {
            max_repository_bytes: parse_u64(
                "max-repository-bytes",
                self.one("max-repository-bytes")?,
                defaults.max_repository_bytes,
            )?,
            max_decoded_documents: parse_usize(
                "max-decoded-documents",
                self.one("max-decoded-documents")?,
                defaults.max_decoded_documents,
            )?,
            max_samples_per_signature: parse_usize(
                "max-samples-per-signature",
                self.one("max-samples-per-signature")?,
                defaults.max_samples_per_signature,
            )?,
            max_unique_shapes: parse_usize(
                "max-unique-shapes",
                self.one("max-unique-shapes")?,
                defaults.max_unique_shapes,
            )?,
            max_candidate_primitives: parse_usize(
                "max-candidate-primitives",
                self.one("max-candidate-primitives")?,
                defaults.max_candidate_primitives,
            )?,
            max_unresolved_documents: parse_usize(
                "max-unresolved-documents",
                self.one("max-unresolved-documents")?,
                defaults.max_unresolved_documents,
            )?,
        })
    }

    fn key_values(&self, name: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
        let mut mapped_values = std::collections::BTreeMap::new();
        for value in self.many(name) {
            let (key, mapped) = value
                .split_once('=')
                .ok_or_else(|| format!("--{name} value {value:?} must use KEY=VALUE syntax"))?;
            if key.is_empty() || mapped.is_empty() {
                return Err(format!(
                    "--{name} value {value:?} must have non-empty key and value"
                ));
            }
            if mapped_values
                .insert(key.to_owned(), mapped.to_owned())
                .is_some()
            {
                return Err(format!("--{name} defines key {key:?} more than once"));
            }
        }
        Ok(mapped_values)
    }

    fn reject_unknown(&self, allowed: &[&str]) -> Result<(), String> {
        if let Some((name, _)) = self
            .values
            .iter()
            .find(|(name, _)| !allowed.contains(&name.as_str()))
        {
            Err(format!("unknown option --{name}"))
        } else {
            Ok(())
        }
    }
}

fn parse_u64(name: &str, value: Option<String>, default: u64) -> Result<u64, String> {
    value.map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|error| format!("invalid --{name} value {value:?}: {error}"))
    })
}

fn parse_usize(name: &str, value: Option<String>, default: usize) -> Result<usize, String> {
    value.map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|error| format!("invalid --{name} value {value:?}: {error}"))
    })
}

fn parse_profile(value: Option<String>) -> Result<crate::model::SurveyProfile, String> {
    match value.as_deref().unwrap_or("generic") {
        "generic" => Ok(crate::model::SurveyProfile::Generic),
        "foundry-dnd5e" => Ok(crate::model::SurveyProfile::FoundryDnd5e),
        "foundry-pf2e" => Ok(crate::model::SurveyProfile::FoundryPf2e),
        profile => Err(format!(
            "unknown survey profile {profile:?}; expected generic, foundry-dnd5e, or foundry-pf2e"
        )),
    }
}

fn usage() -> String {
    "Usage:
  rpg-primitive-survey inventory --study ID --repository URL [--ref main] [--manifest system.json] [--profile generic|foundry-dnd5e|foundry-pf2e] [--content-root PATH] [--source-pointer JSON_POINTER] [--source-fallback PACK=SOURCE] [--work-root PATH] [limit options]
  rpg-primitive-survey scan --study ID --include-source SOURCE [--include-source SOURCE] [--work-root PATH] [limit options]
  rpg-primitive-survey tracked-audit [--repository-root PATH] [--max-tracked-bytes N]

Limit options:
  --max-repository-bytes N
  --max-decoded-documents N
  --max-samples-per-signature N
  --max-unique-shapes N
  --max-candidate-primitives N
  --max-unresolved-documents N"
        .to_owned()
}
