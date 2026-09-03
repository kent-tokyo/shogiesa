use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const RECIPE_VERSION: u32 = 1;
const PLAN_VERSION: u32 = 1;

#[derive(clap::Args)]
pub(crate) struct RecipeArgs {
    #[command(subcommand)]
    action: RecipeAction,
}

#[derive(clap::Subcommand)]
enum RecipeAction {
    /// Validate a recipe and print its dependency/identity plan; executes no stages
    Plan(RecipePlanArgs),
}

#[derive(clap::Args)]
struct RecipePlanArgs {
    /// Typed recipe JSON file
    #[arg(long)]
    recipe: PathBuf,
    /// Write the complete deterministic plan as JSON
    #[arg(long)]
    json_out: Option<PathBuf>,
}

pub(crate) fn run(args: RecipeArgs) -> Result<()> {
    match args.action {
        RecipeAction::Plan(args) => plan(args),
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum RecipeCommand {
    Extract,
    Label,
    Stability,
    Pack,
    Unpack,
    Split,
    Sample,
    Mine,
    Balance,
    Stratify,
    Select,
    Filter,
    Calibrate,
    Audit,
    Tune,
    ConflictReport,
    BlockReport,
    Report,
    DatasetDiff,
    Distribution,
    Validate,
    FromMatch,
    MergeObservations,
    MakeGateOpenings,
    Shuffle,
}

impl RecipeCommand {
    fn as_str(self) -> &'static str {
        match self {
            Self::Extract => "extract",
            Self::Label => "label",
            Self::Stability => "stability",
            Self::Pack => "pack",
            Self::Unpack => "unpack",
            Self::Split => "split",
            Self::Sample => "sample",
            Self::Mine => "mine",
            Self::Balance => "balance",
            Self::Stratify => "stratify",
            Self::Select => "select",
            Self::Filter => "filter",
            Self::Calibrate => "calibrate",
            Self::Audit => "audit",
            Self::Tune => "tune",
            Self::ConflictReport => "conflict-report",
            Self::BlockReport => "block-report",
            Self::Report => "report",
            Self::DatasetDiff => "dataset-diff",
            Self::Distribution => "distribution",
            Self::Validate => "validate",
            Self::FromMatch => "from-match",
            Self::MergeObservations => "merge-observations",
            Self::MakeGateOpenings => "make-gate-openings",
            Self::Shuffle => "shuffle",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RecipeSpec {
    recipe_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    stages: Vec<RecipeStage>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RecipeStage {
    id: String,
    command: RecipeCommand,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    outputs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RecipePlan {
    plan_version: u32,
    recipe_version: u32,
    recipe_path: String,
    recipe_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    stages: Vec<PlannedStage>,
    summary: PlanSummary,
    executed: bool,
}

#[derive(Debug, Serialize)]
struct PlannedStage {
    index: usize,
    id: String,
    command: String,
    args: Vec<String>,
    inputs: Vec<PlannedInput>,
    outputs: Vec<String>,
    dependencies: Vec<String>,
    stage_identity: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct PlannedInput {
    path: String,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    producer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
    available: bool,
}

#[derive(Debug, Default, Serialize)]
struct PlanSummary {
    stages: usize,
    ready: usize,
    waiting_for_dependencies: usize,
    blocked_missing_input: usize,
    blocked_dependency: usize,
}

fn hash_file(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(
        File::open(path).with_context(|| format!("cannot open recipe input {path:?}"))?,
    );
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("cannot read recipe input {path:?}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn resolve(recipe_dir: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        recipe_dir.join(path)
    }
}

fn normalized_recipe_path(recipe_dir: &Path, raw: &str) -> PathBuf {
    resolve(recipe_dir, raw).components().collect()
}

fn valid_stage_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_recipe(spec: &RecipeSpec, recipe_dir: &Path) -> Result<BTreeMap<PathBuf, usize>> {
    if spec.recipe_version != RECIPE_VERSION {
        bail!(
            "unsupported recipe_version {}; expected {RECIPE_VERSION}",
            spec.recipe_version
        );
    }
    if spec.stages.is_empty() {
        bail!("recipe must contain at least one stage");
    }

    let mut stage_ids = BTreeSet::new();
    let mut output_producers = BTreeMap::new();
    for (index, stage) in spec.stages.iter().enumerate() {
        if !valid_stage_id(&stage.id) {
            bail!(
                "invalid stage id {:?}; use ASCII letters, digits, '-' or '_'",
                stage.id
            );
        }
        if !stage_ids.insert(stage.id.clone()) {
            bail!("duplicate stage id {:?}", stage.id);
        }
        if stage
            .args
            .iter()
            .chain(stage.inputs.iter())
            .chain(stage.outputs.iter())
            .any(|value| value.contains('\0'))
        {
            bail!("stage {:?} contains a NUL byte", stage.id);
        }
        if stage.inputs.iter().any(String::is_empty) || stage.outputs.iter().any(String::is_empty) {
            bail!("stage {:?} contains an empty input/output path", stage.id);
        }
        let inputs: BTreeSet<PathBuf> = stage
            .inputs
            .iter()
            .map(|path| normalized_recipe_path(recipe_dir, path))
            .collect();
        if let Some(path) = stage
            .outputs
            .iter()
            .find(|output| inputs.contains(&normalized_recipe_path(recipe_dir, output)))
        {
            bail!(
                "stage {:?} uses {:?} as both input and output",
                stage.id,
                path
            );
        }
        for output in &stage.outputs {
            if Path::new(output).is_absolute()
                || Path::new(output)
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                bail!(
                    "stage {:?} output {:?} escapes the recipe directory",
                    stage.id,
                    output
                );
            }
            let normalized = normalized_recipe_path(recipe_dir, output);
            if let Some(previous) = output_producers.insert(normalized, index) {
                bail!(
                    "output {:?} is produced by both {:?} and {:?}",
                    output,
                    spec.stages[previous].id,
                    stage.id
                );
            }
        }
    }
    Ok(output_producers)
}

fn stage_identity(
    stage: &RecipeStage,
    inputs: &[PlannedInput],
    dependency_identities: &BTreeMap<String, String>,
) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&serde_json::to_vec(stage)?);
    for input in inputs {
        if let Some(hash) = &input.content_hash {
            hasher.update(hash.as_bytes());
        }
        if let Some(producer) = &input.producer
            && let Some(identity) = dependency_identities.get(producer)
        {
            hasher.update(identity.as_bytes());
        }
        if !input.available {
            hasher.update(b"missing");
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn build_plan(recipe_path: &Path, spec: RecipeSpec, recipe_hash: String) -> Result<RecipePlan> {
    let recipe_dir = recipe_path.parent().unwrap_or_else(|| Path::new("."));
    let output_producers = validate_recipe(&spec, recipe_dir)?;
    let mut planned = Vec::<PlannedStage>::new();
    let mut identities = BTreeMap::<String, String>::new();
    let mut statuses = BTreeMap::<String, &'static str>::new();
    let mut summary = PlanSummary {
        stages: spec.stages.len(),
        ..PlanSummary::default()
    };
    let stage_ids: Vec<String> = spec.stages.iter().map(|stage| stage.id.clone()).collect();

    for (index, stage) in spec.stages.into_iter().enumerate() {
        let mut inputs = Vec::new();
        let mut dependencies = BTreeSet::new();
        let mut missing_input = false;
        for raw in &stage.inputs {
            let resolved = normalized_recipe_path(recipe_dir, raw);
            if let Some(&producer_index) = output_producers.get(&resolved) {
                if producer_index >= index {
                    bail!(
                        "stage {:?} input {:?} is produced by later stage {:?}; stages must be topologically ordered",
                        stage.id,
                        raw,
                        stage_ids[producer_index]
                    );
                }
                let producer = stage_ids[producer_index].clone();
                dependencies.insert(producer.clone());
                inputs.push(PlannedInput {
                    path: resolved.display().to_string(),
                    source: "stage",
                    producer: Some(producer),
                    content_hash: None,
                    available: false,
                });
            } else {
                let available = resolved.is_file();
                let content_hash = available.then(|| hash_file(&resolved)).transpose()?;
                missing_input |= !available;
                inputs.push(PlannedInput {
                    path: resolved.display().to_string(),
                    source: "external",
                    producer: None,
                    content_hash,
                    available,
                });
            }
        }
        let dependencies: Vec<String> = dependencies.into_iter().collect();
        let dependency_blocked = dependencies.iter().any(|dependency| {
            statuses
                .get(dependency)
                .is_some_and(|status| status.starts_with("blocked"))
        });
        let status = if missing_input {
            summary.blocked_missing_input += 1;
            "blocked-missing-input"
        } else if dependency_blocked {
            summary.blocked_dependency += 1;
            "blocked-dependency"
        } else if dependencies.is_empty() {
            summary.ready += 1;
            "ready"
        } else {
            summary.waiting_for_dependencies += 1;
            "waiting-for-dependencies"
        };
        let outputs: Vec<String> = stage
            .outputs
            .iter()
            .map(|path| {
                normalized_recipe_path(recipe_dir, path)
                    .display()
                    .to_string()
            })
            .collect();
        let identity = stage_identity(&stage, &inputs, &identities)?;
        identities.insert(stage.id.clone(), identity.clone());
        statuses.insert(stage.id.clone(), status);
        planned.push(PlannedStage {
            index: index + 1,
            id: stage.id,
            command: stage.command.as_str().to_string(),
            args: stage.args,
            inputs,
            outputs,
            dependencies,
            stage_identity: identity,
            status,
        });
    }

    Ok(RecipePlan {
        plan_version: PLAN_VERSION,
        recipe_version: spec.recipe_version,
        recipe_path: recipe_path.display().to_string(),
        recipe_hash,
        name: spec.name,
        stages: planned,
        summary,
        executed: false,
    })
}

fn plan(args: RecipePlanArgs) -> Result<()> {
    let bytes =
        fs::read(&args.recipe).with_context(|| format!("cannot read recipe {:?}", args.recipe))?;
    let spec: RecipeSpec = serde_json::from_slice(&bytes)
        .with_context(|| format!("cannot parse recipe {:?}", args.recipe))?;
    let recipe_hash = blake3::hash(&bytes).to_hex().to_string();
    let plan = build_plan(&args.recipe, spec, recipe_hash)?;

    if let Some(path) = &args.json_out {
        fs::write(path, serde_json::to_string_pretty(&plan)? + "\n")
            .with_context(|| format!("cannot write {path:?}"))?;
    }

    let mut out = String::new();
    writeln!(out, "recipe plan").unwrap();
    writeln!(out, "recipe             : {}", plan.recipe_path).unwrap();
    writeln!(out, "recipe version     : {}", plan.recipe_version).unwrap();
    writeln!(out, "stages             : {}", plan.summary.stages).unwrap();
    for stage in &plan.stages {
        writeln!(
            out,
            "  [{}] {} ({}) — {}",
            stage.index, stage.id, stage.command, stage.status
        )
        .unwrap();
        if !stage.dependencies.is_empty() {
            writeln!(out, "      depends on: {}", stage.dependencies.join(", ")).unwrap();
        }
    }
    writeln!(out, "ready              : {}", plan.summary.ready).unwrap();
    writeln!(
        out,
        "waiting            : {}",
        plan.summary.waiting_for_dependencies
    )
    .unwrap();
    writeln!(
        out,
        "blocked missing    : {}",
        plan.summary.blocked_missing_input
    )
    .unwrap();
    writeln!(
        out,
        "blocked dependency : {}",
        plan.summary.blocked_dependency
    )
    .unwrap();
    writeln!(out, "executed           : no").unwrap();
    print!("{out}");
    Ok(())
}
