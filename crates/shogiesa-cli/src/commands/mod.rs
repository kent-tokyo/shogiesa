pub(super) mod cache;
pub(super) mod diagnostics;
pub(super) mod ingest;
pub(super) mod quality;
pub(super) mod selection;
pub(super) mod transport;

pub(super) use cache::{cmd_cache_prune, cmd_cache_stats, cmd_cache_verify};
pub(super) use diagnostics::{
    cmd_block_report, cmd_conflict_report, cmd_dataset_diff, cmd_distribution, cmd_report,
    cmd_validate,
};
pub(super) use ingest::{
    ResumeIndex, build_resume_index, cmd_extract, cmd_from_match, cmd_merge_observations,
    merge_alignment_key, merge_resume_observations, read_resume_observations_at,
};
pub(super) use quality::{cmd_audit, cmd_calibrate, cmd_filter, cmd_tune};
pub(super) use selection::{
    cmd_balance, cmd_lineprior_export, cmd_make_gate_openings, cmd_mine, cmd_sample, cmd_select,
    cmd_shuffle, cmd_stratify, is_legal_bestmove_lite,
};
pub(super) use transport::{cmd_pack, cmd_quality_report, cmd_unpack};
