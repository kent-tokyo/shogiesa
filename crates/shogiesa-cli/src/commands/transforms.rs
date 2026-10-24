use super::super::*;
use super::label::hash_parts_u64;

pub(crate) fn cmd_stability(args: StabilityArgs) -> Result<()> {
    let mut writer = AtomicWriter::new(&args.out)?;

    let mut total = 0usize;
    let mut enriched = 0usize;
    let mut skipped = 0usize;

    for item in open_position_jsonl(&args.input)? {
        total += 1;
        let Some(mut rec) = permissive_position_record(item, &args.input, &mut skipped)? else {
            continue;
        };
        rec.fill_stability();
        if rec.stability.is_some() {
            enriched += 1;
        }
        serde_json::to_writer(&mut writer, &rec)?;
        writer.write_all(b"\n")?;
    }

    writer.commit()?;
    eprintln!(
        "done: {total} read, {enriched} enriched with stability, {skipped} skipped → {:?}",
        args.out
    );
    Ok(())
}

/// One resident file handle in `WriterPool`, tagged with the tick it was last written at so the
/// pool can find its least-recently-used entry in a bounded linear scan (fine at ≤`max_open`
/// entries -- a dedicated LRU crate would be over-engineering for this bound).
struct WriterPoolEntry {
    writer: BufWriter<File>,
    last_used: u64,
}

/// Bounded pool of per-source output file handles for `split --by-source`, so a corpus with far
/// more distinct source games than a process's FD limit doesn't exhaust it. On a miss at
/// capacity, evicts (and explicitly flushes -- so a write error surfaces here, not silently
/// swallowed by `BufWriter`'s `Drop`) whichever resident file was written to longest ago, then
/// opens the target. `opened_this_run` is tracked separately from the (evictable) `writers` map:
/// a source's *first* touch this run truncates (matching this command's previous fresh-file
/// behavior), but every later open -- whether still resident or evicted-and-reopened -- must
/// append, or a re-open after eviction would silently discard everything written before it.
struct WriterPool {
    max_open: usize,
    writers: HashMap<String, WriterPoolEntry>,
    opened_this_run: HashSet<String>,
    tick: u64,
}

impl WriterPool {
    fn new(max_open: usize) -> Self {
        Self {
            max_open,
            writers: HashMap::new(),
            opened_this_run: HashSet::new(),
            tick: 0,
        }
    }

    fn write_line(&mut self, out_path: &Path, key: &str, rec: &PositionRecord) -> Result<()> {
        self.tick += 1;
        let tick = self.tick;
        if let Some(entry) = self.writers.get_mut(key) {
            serde_json::to_writer(&mut entry.writer, rec)?;
            entry.writer.write_all(b"\n")?;
            entry.last_used = tick;
            return Ok(());
        }

        if self.writers.len() >= self.max_open {
            let lru_key = self
                .writers
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
                .expect("writers is non-empty when at capacity");
            let mut evicted = self.writers.remove(&lru_key).unwrap();
            evicted.writer.flush()?;
        }

        let first_touch = self.opened_this_run.insert(key.to_string());
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(first_touch)
            .append(!first_touch)
            .open(out_path)
            .with_context(|| format!("cannot open {out_path:?}"))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, rec)?;
        writer.write_all(b"\n")?;
        self.writers.insert(
            key.to_string(),
            WriterPoolEntry {
                writer,
                last_used: tick,
            },
        );
        Ok(())
    }

    fn flush_all(&mut self) -> Result<()> {
        for entry in self.writers.values_mut() {
            entry.writer.flush()?;
        }
        Ok(())
    }
}

struct SplitBundleEntry {
    staged: PathBuf,
    destination: PathBuf,
    prepared_temp: Option<tempfile::TempPath>,
    backup: Option<tempfile::TempPath>,
    committed: bool,
}

fn rollback_split_bundle(entries: &mut [SplitBundleEntry]) -> Vec<String> {
    let mut errors = Vec::new();
    for entry in entries.iter_mut().rev() {
        if entry.committed {
            if let Err(error) = fs::remove_file(&entry.destination) {
                let mut message = format!(
                    "cannot remove partially committed {:?}: {error}",
                    entry.destination
                );
                if let Some(backup) = entry.backup.take() {
                    let backup_path = backup.to_path_buf();
                    match backup.keep() {
                        Ok(path) => {
                            message.push_str(&format!("; previous output retained at {path:?}"));
                        }
                        Err(keep_error) => {
                            message.push_str(&format!(
                                "; could not retain previous output at {backup_path:?}: {keep_error}"
                            ));
                        }
                    }
                }
                errors.push(message);
                continue;
            }
            entry.committed = false;
        }
        if let Some(backup) = entry.backup.take() {
            let backup_path = backup.to_path_buf();
            if let Err(error) = fs::rename(&backup_path, &entry.destination) {
                let retention = match backup.keep() {
                    Ok(path) => format!("previous output retained at {path:?}"),
                    Err(keep_error) => {
                        format!("could not retain previous output at {backup_path:?}: {keep_error}")
                    }
                };
                errors.push(format!(
                    "cannot restore backup for {:?}: {error}; {retention}",
                    entry.destination,
                ));
            }
        }
    }
    errors
}

/// Commit all per-source files and their manifest as one recoverable bundle. Each destination is
/// backed up before replacement; any later failure restores every earlier destination in reverse
/// order. Existing symlinks are resolved so their targets are updated without replacing the link.
fn commit_split_bundle(staged_files: &[(PathBuf, PathBuf)]) -> Result<()> {
    commit_split_bundle_with(staged_files, |_| Ok(()))
}

fn commit_split_bundle_with<F>(
    staged_files: &[(PathBuf, PathBuf)],
    mut before_commit: F,
) -> Result<()>
where
    F: FnMut(usize) -> Result<()>,
{
    let mut entries = Vec::with_capacity(staged_files.len());
    let mut destinations = HashSet::with_capacity(staged_files.len());

    // Complete every fallible preparation step before moving an existing destination. A symlink
    // target may live on another filesystem, so copy that staged file to a temporary neighbor;
    // the final replacement can then use a same-filesystem rename.
    for (staged, requested_destination) in staged_files {
        let destination = match fs::symlink_metadata(requested_destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                fs::canonicalize(requested_destination).with_context(|| {
                    format!("cannot resolve split output symlink {requested_destination:?}")
                })?
            }
            Ok(metadata) if metadata.is_dir() => {
                anyhow::bail!("split output destination is a directory: {requested_destination:?}")
            }
            Ok(_) => requested_destination.clone(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                requested_destination.clone()
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot inspect split output {requested_destination:?}")
                });
            }
        };
        if !destinations.insert(destination.clone()) {
            anyhow::bail!("split outputs resolve to the same destination: {destination:?}");
        }

        let mut prepared_temp = None;
        let prepared_staged = if destination != *requested_destination {
            let parent = destination.parent().unwrap_or(Path::new("."));
            let temp = tempfile::NamedTempFile::new_in(parent).with_context(|| {
                format!("cannot stage split output beside symlink target {destination:?}")
            })?;
            fs::copy(staged, temp.path()).with_context(|| {
                format!("cannot stage split output for symlink target {destination:?}")
            })?;
            temp.as_file()
                .sync_all()
                .with_context(|| format!("cannot sync staged split output for {destination:?}"))?;
            let temp = temp.into_temp_path();
            let path = temp.to_path_buf();
            prepared_temp = Some(temp);
            path
        } else {
            // Windows requires a writable handle for `sync_all`/`FlushFileBuffers` even when
            // the file contents are already complete. Opening read-only made bundle commits
            // fail during preparation before rollback behavior could run.
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(staged)
                .with_context(|| format!("cannot open staged split output {staged:?}"))?
                .sync_all()
                .with_context(|| format!("cannot sync staged split output {staged:?}"))?;
            staged.clone()
        };

        entries.push(SplitBundleEntry {
            staged: prepared_staged,
            destination,
            prepared_temp,
            backup: None,
            committed: false,
        });
    }

    for index in 0..entries.len() {
        if let Err(error) = before_commit(index) {
            let rollback_errors = rollback_split_bundle(&mut entries);
            let suffix = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("; rollback incomplete: {}", rollback_errors.join("; "))
            };
            anyhow::bail!("split bundle commit interrupted: {error}{suffix}");
        }
        let destination = entries[index].destination.clone();
        if destination.exists() {
            let parent = destination.parent().unwrap_or(Path::new("."));
            let backup = match tempfile::NamedTempFile::new_in(parent) {
                Ok(file) => file.into_temp_path(),
                Err(error) => {
                    let rollback_errors = rollback_split_bundle(&mut entries);
                    let suffix = if rollback_errors.is_empty() {
                        String::new()
                    } else {
                        format!("; rollback incomplete: {}", rollback_errors.join("; "))
                    };
                    anyhow::bail!(
                        "cannot create backup beside split output {destination:?}: {error}{suffix}"
                    );
                }
            };
            let backup_path = backup.to_path_buf();
            if let Err(error) =
                fs::remove_file(&backup_path).and_then(|()| fs::rename(&destination, &backup_path))
            {
                let rollback_errors = rollback_split_bundle(&mut entries);
                let suffix = if rollback_errors.is_empty() {
                    String::new()
                } else {
                    format!("; rollback incomplete: {}", rollback_errors.join("; "))
                };
                anyhow::bail!("cannot back up split output {destination:?}: {error}{suffix}");
            }
            entries[index].backup = Some(backup);
        }

        if let Err(error) = fs::rename(&entries[index].staged, &destination) {
            let rollback_errors = rollback_split_bundle(&mut entries);
            let suffix = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("; rollback incomplete: {}", rollback_errors.join("; "))
            };
            anyhow::bail!("cannot commit split output {destination:?}: {error}{suffix}");
        }
        entries[index].committed = true;
    }

    for entry in &mut entries {
        if let Some(backup) = entry.backup.take()
            && let Err(error) = backup.close()
        {
            tracing::warn!(
                destination = ?entry.destination,
                "cannot remove old split output backup: {error}"
            );
        }
        // Keep the guard alive through the rename; dropping it now only tries to remove the old,
        // no-longer-existing temporary path.
        entry.prepared_temp.take();
    }
    Ok(())
}

/// Keep the long-standing readable file name when it is unique, but disambiguate two source
/// paths that sanitize to the same name (`a/b` and `a_b`, for example). The suffix is based on
/// the original source identity, so it is deterministic and does not depend on process state.
fn allocate_split_file_name(source: &str, owners: &mut HashMap<String, String>) -> Result<String> {
    let safe: String = source
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let plain = format!("{safe}.jsonl");
    let file_name = match owners.get(&plain) {
        None => plain,
        Some(owner) if owner == source => plain,
        Some(_) => {
            let digest = blake3::hash(source.as_bytes()).to_hex();
            format!("{safe}--{}.jsonl", &digest[..16])
        }
    };
    if let Some(owner) = owners.get(&file_name)
        && owner != source
    {
        anyhow::bail!(
            "source paths {owner:?} and {source:?} map to the same split output {file_name:?}"
        );
    }
    owners.insert(file_name.clone(), source.to_string());
    Ok(file_name)
}

pub(crate) fn cmd_split(args: SplitArgs) -> Result<()> {
    if args.train.is_some() || args.valid.is_some() || args.test.is_some() {
        return cmd_split_train_valid_test(args);
    }

    if !args.by_source {
        anyhow::bail!("either --by-source --out-dir, or --train/--valid/--test, is required");
    }
    let out_dir = args
        .out_dir
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--out-dir is required with --by-source"))?;
    fs::create_dir_all(out_dir).with_context(|| format!("cannot create {out_dir:?}"))?;
    let manifest_path = out_dir.join("manifest.json");
    let staging = tempfile::Builder::new()
        .prefix(".shogiesa-split-")
        .tempdir_in(out_dir)
        .with_context(|| format!("cannot create split staging directory in {out_dir:?}"))?;
    let staging_dir = staging.path();

    // Group records by source path, writing into per-source output files via a bounded LRU pool
    let mut pool = WriterPool::new(args.max_open_writers.max(1));
    let mut file_names: HashMap<String, String> = HashMap::new();
    let mut file_name_owners: HashMap<String, String> = HashMap::new();
    let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut broken = 0usize;

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };

        let key = rec.source.path.clone();
        let file_name = match file_names.get(&key) {
            Some(file_name) => file_name.clone(),
            None => {
                let file_name = allocate_split_file_name(&key, &mut file_name_owners)?;
                file_names.insert(key.clone(), file_name.clone());
                file_name
            }
        };
        let out_path = out_dir.join(&file_name);
        validate_path_roles(
            &[("--input", &args.input)],
            &[
                ("split output", &out_path),
                ("split manifest", &manifest_path),
            ],
        )?;
        pool.write_line(&staging_dir.join(&file_name), &key, &rec)?;
        *file_counts.entry(file_name).or_default() += 1;
        total += 1;
    }
    pool.flush_all()?;
    drop(pool);

    let output_hashes: BTreeMap<String, String> = file_counts
        .keys()
        .map(|file_name| Ok((file_name.clone(), hash_file(&staging_dir.join(file_name))?)))
        .collect::<Result<_>>()?;

    let manifest = serde_json::json!({
        "shogiesa_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
        "fingerprint_algorithm": "blake3",
        "input": args.input.display().to_string(),
        "input_hash": hash_file(&args.input)?,
        "by_source": args.by_source,
        "total_positions": total,
        "skipped_records": broken,
        "files": file_counts,
        "output_hashes": output_hashes,
    });
    let staged_manifest = staging_dir.join("manifest.json");
    atomic_write_bytes(
        &staged_manifest,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )
    .with_context(|| format!("cannot write staged split manifest {staged_manifest:?}"))?;

    let mut staged_files: Vec<(PathBuf, PathBuf)> = file_counts
        .keys()
        .map(|file_name| (staging_dir.join(file_name), out_dir.join(file_name)))
        .collect();
    // The manifest is committed last, so readers never see it claim a new bundle before every
    // referenced file is in place. A failure still rolls the earlier file replacements back.
    staged_files.push((staged_manifest, manifest_path));
    commit_split_bundle(&staged_files)?;

    eprintln!(
        "done: {total} positions split, {broken} skipped, into {} files → {:?}",
        file_counts.len(),
        out_dir
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SplitBucket {
    Test,
    Valid,
    Train,
}

/// Strip a KIF variation suffix (`#varN@ply`, from shogiesa-kif's `extract_from_str`) so a
/// variation's positions hash into the same split bucket as the mainline game it branched
/// from. A variation only emits moves from its branch ply onward, so what's at stake isn't
/// identical positions landing in different splits -- it's *correlation* between siblings:
/// mainline-ply-N and variation-ply-N share the exact same parent position and near-identical
/// context. Splitting correlated siblings across train/valid/test lets the model be evaluated
/// on positions correlated with ones it trained on, inflating validation results.
fn split_root_path(source_path: &str) -> &str {
    source_path.split("#var").next().unwrap_or(source_path)
}

/// The grouping key used to keep a game's mainline and its variations in the same split bucket.
/// Prefers `source.root_id` (set by extractors that produce variations, e.g. shogiesa-kif) since
/// it doesn't depend on parsing a string convention back out of `path`; falls back to
/// `split_root_path` for JSONL/extractors (CSA, or anything predating this field) that never set
/// `root_id`, preserving old behavior exactly for that data.
pub(crate) fn split_root_key(source: &SourceInfo) -> &str {
    source
        .root_id
        .as_deref()
        .unwrap_or_else(|| split_root_path(&source.path))
}

pub(super) fn assign_split_bucket(
    seed: u64,
    root_key: &str,
    valid_frac: f64,
    test_frac: f64,
) -> SplitBucket {
    let seed_bytes = seed.to_le_bytes();
    let unit = hash_parts_u64(&[&seed_bytes, root_key.as_bytes()]) as f64 / u64::MAX as f64;
    if unit < test_frac {
        SplitBucket::Test
    } else if unit < test_frac + valid_frac {
        SplitBucket::Valid
    } else {
        SplitBucket::Train
    }
}

fn cmd_split_train_valid_test(args: SplitArgs) -> Result<()> {
    let (Some(train_path), Some(valid_path), Some(test_path)) =
        (&args.train, &args.valid, &args.test)
    else {
        anyhow::bail!("--train, --valid, and --test must all be provided together");
    };
    if args.valid_frac + args.test_frac >= 1.0 {
        anyhow::bail!("--valid-frac + --test-frac must be < 1.0");
    }

    let mut train_writer = AtomicWriter::new(train_path)?;
    let mut valid_writer = AtomicWriter::new(valid_path)?;
    let mut test_writer = AtomicWriter::new(test_path)?;

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sources: HashMap<&'static str, HashSet<String>> = HashMap::new();
    let mut roots: HashMap<&'static str, HashSet<String>> = HashMap::new();
    let mut total = 0usize;
    let mut broken = 0usize;

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };

        let bucket = assign_split_bucket(
            args.seed,
            split_root_key(&rec.source),
            args.valid_frac,
            args.test_frac,
        );
        let (name, writer) = match bucket {
            SplitBucket::Train => ("train", &mut train_writer),
            SplitBucket::Valid => ("valid", &mut valid_writer),
            SplitBucket::Test => ("test", &mut test_writer),
        };
        serde_json::to_writer(&mut *writer, &rec)?;
        writer.write_all(b"\n")?;
        *counts.entry(name).or_default() += 1;
        let root_key = split_root_key(&rec.source).to_string();
        sources.entry(name).or_default().insert(rec.source.path);
        roots.entry(name).or_default().insert(root_key);
        total += 1;
    }

    train_writer.commit()?;
    valid_writer.commit()?;
    test_writer.commit()?;

    let split_manifest = |name: &str, path: &PathBuf, output_hash: &str| {
        let mut source_root_ids: Vec<String> = roots
            .get(name)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        source_root_ids.sort();
        serde_json::json!({
            "path": path,
            "output_hash": output_hash,
            "positions": counts.get(name).copied().unwrap_or(0),
            "sources": sources.get(name).map(HashSet::len).unwrap_or(0),
            "source_roots": roots.get(name).map(HashSet::len).unwrap_or(0),
            "source_root_ids": source_root_ids,
        })
    };
    let train_hash = hash_file(train_path)?;
    let valid_hash = hash_file(valid_path)?;
    let test_hash = hash_file(test_path)?;
    let manifest = serde_json::json!({
        "shogiesa_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
        "fingerprint_algorithm": "blake3",
        "input": args.input.display().to_string(),
        "input_hash": hash_file(&args.input)?,
        "seed": args.seed,
        "requested": { "valid_frac": args.valid_frac, "test_frac": args.test_frac },
        "total_positions": total,
        "skipped_records": broken,
        "splits": {
            "train": split_manifest("train", train_path, &train_hash),
            "valid": split_manifest("valid", valid_path, &valid_hash),
            "test": split_manifest("test", test_path, &test_hash),
        },
    });
    let manifest_path = train_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("manifest.json");
    atomic_write_bytes(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )
    .with_context(|| format!("cannot write {manifest_path:?}"))?;

    eprintln!(
        "done: {total} positions split, {broken} skipped (seed={}) → train={}, valid={}, test={}",
        args.seed,
        counts.get("train").copied().unwrap_or(0),
        counts.get("valid").copied().unwrap_or(0),
        counts.get("test").copied().unwrap_or(0),
    );
    Ok(())
}

#[cfg(test)]
mod atomic_writer_tests {
    use super::*;

    #[test]
    fn drop_without_commit_preserves_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.bin");
        fs::write(&path, b"previous").unwrap();

        {
            let mut writer = AtomicWriter::new(&path).unwrap();
            writer.write_all(b"partial replacement").unwrap();
        }

        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn commit_replaces_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.bin");
        fs::write(&path, b"previous").unwrap();

        let mut writer = AtomicWriter::new(&path).unwrap();
        writer.write_all(b"complete replacement").unwrap();
        writer.commit().unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"complete replacement");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn commit_follows_existing_output_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.bin");
        let link = dir.path().join("output.bin");
        fs::write(&target, b"previous").unwrap();
        symlink(&target, &link).unwrap();

        let mut writer = AtomicWriter::new(&link).unwrap();
        writer.write_all(b"replacement").unwrap();
        writer.commit().unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn split_bundle_rolls_back_earlier_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let staged_a = dir.path().join("staged-a");
        let staged_b = dir.path().join("staged-b");
        let destination_a = dir.path().join("destination-a");
        let destination_b = dir.path().join("destination-b");
        fs::write(&staged_a, b"new-a").unwrap();
        fs::write(&staged_b, b"new-b").unwrap();
        fs::write(&destination_a, b"old-a").unwrap();
        fs::write(&destination_b, b"old-b").unwrap();

        let error = commit_split_bundle_with(
            &[
                (staged_a, destination_a.clone()),
                (staged_b, destination_b.clone()),
            ],
            |index| {
                if index == 1 {
                    anyhow::bail!("injected failure");
                }
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("injected failure"));
        assert_eq!(fs::read(destination_a).unwrap(), b"old-a");
        assert_eq!(fs::read(destination_b).unwrap(), b"old-b");
    }

    #[test]
    fn failed_split_bundle_rollback_retains_previous_output_backup() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let mut backup = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        backup.write_all(b"previous output").unwrap();
        backup.as_file().sync_all().unwrap();
        let backup_path = backup.path().to_path_buf();

        let mut entries = [SplitBundleEntry {
            staged: dir.path().join("staged"),
            destination,
            prepared_temp: None,
            backup: Some(backup.into_temp_path()),
            committed: true,
        }];
        let errors = rollback_split_bundle(&mut entries);

        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("previous output retained"));
        assert_eq!(fs::read(backup_path).unwrap(), b"previous output");
    }
}
