use super::super::*;

pub(crate) fn cmd_pack(args: PackArgs) -> Result<()> {
    let mut reader = open_hashed_position_jsonl(&args.input)?;
    let mut writer = AtomicWriter::new(&args.out)?;

    pack::write_header(&mut writer)?;

    let mut total = 0usize;
    let mut skipped = 0usize;
    let mut manifest = args
        .manifest
        .is_some()
        .then(|| new_run_manifest("pack", &args.input));
    for item in &mut reader {
        let Some(rec) = permissive_position_record(item, &args.input, &mut skipped)? else {
            continue;
        };
        pack::encode_record(&rec, &mut writer)?;
        total += 1;
        if let Some(m) = &mut manifest {
            accumulate_coverage(m, std::slice::from_ref(&rec));
        }
    }
    let (input_hash, _exact_sha256) = finish_hashed_position_jsonl(reader);
    writer.commit()?;
    eprintln!("done: {total} packed, {skipped} skipped → {:?}", args.out);
    if let (Some(mut m), Some(manifest_path)) = (manifest, &args.manifest) {
        m.input_hash = input_hash;
        m.output_sha256 = Some(hash_file_sha256(&args.out)?);
        m.records_read = total + skipped;
        m.records_kept = total;
        m.records_dropped = skipped;
        write_manifest(manifest_path, &m)?;
    }
    Ok(())
}

pub(crate) fn cmd_unpack(args: UnpackArgs) -> Result<()> {
    let mut reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut writer = AtomicWriter::new(&args.out)?;

    pack::read_header(&mut reader)?;

    let mut total = 0usize;
    loop {
        // An empty input buffer is the only clean end-of-file condition. If bytes remain and
        // decode_record reports UnexpectedEof, the pack is truncated rather than merely empty;
        // do not silently accept a corrupt tail.
        if reader.fill_buf()?.is_empty() {
            break;
        }
        match pack::decode_record(&mut reader) {
            Ok(rec) => {
                serde_json::to_writer(&mut writer, &rec)?;
                writer.write_all(b"\n")?;
                total += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                anyhow::bail!("truncated pack record: {e}")
            }
            Err(e) => return Err(e.into()),
        }
    }
    writer.commit()?;
    eprintln!("done: {total} unpacked → {:?}", args.out);
    Ok(())
}

fn quality_manifest_identity(path: &Path) -> Result<QualityManifestIdentityV1> {
    let bytes = fs::read(path).with_context(|| format!("cannot read manifest {path:?}"))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid JSON in manifest {path:?}"))?;
    let version = value
        .get("manifest_schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .with_context(|| {
            format!("manifest {path:?} has no unsigned 32-bit manifest_schema_version")
        })?;
    Ok(QualityManifestIdentityV1 {
        path: path.display().to_string(),
        sha256: sha256_hex(&bytes),
        manifest_schema_version: version,
    })
}

fn detect_quality_input_format(path: &Path, requested: &str) -> Result<&'static str> {
    match requested {
        "jsonl" => return Ok("jsonl"),
        "pack" => return Ok("pack"),
        "auto" => {}
        _ => unreachable!("clap restricts --input-format"),
    }
    if path
        .extension()
        .is_some_and(|extension| extension == "shgpk")
    {
        return Ok("pack");
    }
    let mut file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    let mut prefix = [0_u8; 8];
    let read = file
        .read(&mut prefix)
        .with_context(|| format!("cannot inspect {path:?}"))?;
    Ok(if read == prefix.len() && &prefix == pack::MAGIC {
        "pack"
    } else {
        "jsonl"
    })
}

fn observe_quality_record(
    record: &PositionRecord,
    shard: &mut QualitySummaryBuilder,
    aggregate: &mut QualitySummaryBuilder,
    shard_hasher: &mut sha2::Sha256,
    aggregate_hasher: &mut sha2::Sha256,
    decoded_bytes: &mut u64,
    schema_versions: &mut BTreeMap<u32, u64>,
) -> Result<()> {
    use sha2::Digest;

    let bytes = serde_json::to_vec(record)?;
    let record_bytes = u64::try_from(bytes.len())?.saturating_add(1);
    shard_hasher.update(&bytes);
    shard_hasher.update(b"\n");
    aggregate_hasher.update(&bytes);
    aggregate_hasher.update(b"\n");
    *decoded_bytes = decoded_bytes.saturating_add(record_bytes);
    *schema_versions.entry(record.schema_version).or_default() += 1;
    shard.observe(record);
    aggregate.observe(record);
    Ok(())
}

fn process_quality_jsonl(
    path: &Path,
    shard: &mut QualitySummaryBuilder,
    aggregate: &mut QualitySummaryBuilder,
    shard_hasher: &mut sha2::Sha256,
    aggregate_hasher: &mut sha2::Sha256,
    decoded_bytes: &mut u64,
    schema_versions: &mut BTreeMap<u32, u64>,
) -> Result<(u64, String)> {
    let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    let mut reader = PositionRecordJsonlReader::new(BufReader::new(Sha256Reader::new(file)));
    for item in &mut reader {
        match item {
            Ok(PositionRecordLine { record, .. }) => observe_quality_record(
                &record,
                shard,
                aggregate,
                shard_hasher,
                aggregate_hasher,
                decoded_bytes,
                schema_versions,
            )?,
            Err(PositionRecordLineError::Parse { source, .. }) => {
                let reason = match source {
                    PositionRecordParseError::Json(_) => "json_invalid_record",
                    PositionRecordParseError::UnsupportedSchema(_) => "json_unsupported_schema",
                    _ => "json_other_parse_error",
                };
                shard.reject(reason);
                aggregate.reject(reason);
            }
            Err(PositionRecordLineError::Read { source, .. }) => {
                return Err(source).with_context(|| format!("cannot read {path:?}"));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(reader.into_inner().into_inner().identity())
}

fn process_quality_pack(
    path: &Path,
    shard: &mut QualitySummaryBuilder,
    aggregate: &mut QualitySummaryBuilder,
    shard_hasher: &mut sha2::Sha256,
    aggregate_hasher: &mut sha2::Sha256,
    decoded_bytes: &mut u64,
    schema_versions: &mut BTreeMap<u32, u64>,
) -> Result<(Option<u16>, u64, String)> {
    let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    let mut reader = BufReader::new(Sha256Reader::new(file));
    let mut header = [0_u8; 10];
    let mut version = None;
    let decode_records = match reader.read_exact(&mut header) {
        Ok(()) => {
            let found = u16::from_le_bytes([header[8], header[9]]);
            version = Some(found);
            if &header[..8] != pack::MAGIC {
                shard.reject("pack_bad_magic");
                aggregate.reject("pack_bad_magic");
                false
            } else if found != pack::FORMAT_VERSION {
                shard.reject("pack_unsupported_version");
                aggregate.reject("pack_unsupported_version");
                false
            } else {
                true
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            shard.reject("pack_truncated_header");
            aggregate.reject("pack_truncated_header");
            false
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read pack header {path:?}"));
        }
    };

    if decode_records {
        loop {
            if reader
                .fill_buf()
                .with_context(|| format!("cannot inspect pack record boundary in {path:?}"))?
                .is_empty()
            {
                break;
            }
            match pack::decode_record(&mut reader) {
                Ok(record) => observe_quality_record(
                    &record,
                    shard,
                    aggregate,
                    shard_hasher,
                    aggregate_hasher,
                    decoded_bytes,
                    schema_versions,
                )?,
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    shard.reject("pack_truncated_record");
                    aggregate.reject("pack_truncated_record");
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    shard.reject("pack_invalid_record");
                    aggregate.reject("pack_invalid_record");
                    break;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot decode pack {path:?}"));
                }
            }
        }
    }
    std::io::copy(&mut reader, &mut std::io::sink())
        .with_context(|| format!("cannot finish hashing pack {path:?}"))?;
    let (input_bytes, input_sha256) = reader.into_inner().identity();
    Ok((version, input_bytes, input_sha256))
}

pub(crate) fn cmd_quality_report(args: QualityReportArgs) -> Result<()> {
    use sha2::Digest;

    if !args.manifest.is_empty() && args.manifest.len() != args.input.len() {
        anyhow::bail!(
            "--manifest must be omitted or repeated exactly once per --input ({} inputs, {} manifests)",
            args.input.len(),
            args.manifest.len()
        );
    }

    let mut aggregate = QualitySummaryBuilder::default();
    let mut aggregate_hasher = sha2::Sha256::new();
    let mut inputs = Vec::with_capacity(args.input.len());
    let mut shards = Vec::new();

    for (index, path) in args.input.iter().enumerate() {
        aggregate.begin_shard();
        let mut shard = QualitySummaryBuilder::default();
        shard.begin_shard();
        let format = detect_quality_input_format(path, &args.input_format)?;
        let metadata = fs::metadata(path).with_context(|| format!("cannot inspect {path:?}"))?;
        if !metadata.is_file() {
            anyhow::bail!("quality-report input must be a file: {path:?}");
        }
        let mut shard_hasher = sha2::Sha256::new();
        let mut decoded_bytes = 0_u64;
        let mut schema_versions = BTreeMap::new();
        let (pack_format_version, input_bytes, input_sha256) = match format {
            "jsonl" => {
                let (input_bytes, input_sha256) = process_quality_jsonl(
                    path,
                    &mut shard,
                    &mut aggregate,
                    &mut shard_hasher,
                    &mut aggregate_hasher,
                    &mut decoded_bytes,
                    &mut schema_versions,
                )?;
                (None, input_bytes, input_sha256)
            }
            "pack" => {
                let (version, input_bytes, input_sha256) = process_quality_pack(
                    path,
                    &mut shard,
                    &mut aggregate,
                    &mut shard_hasher,
                    &mut aggregate_hasher,
                    &mut decoded_bytes,
                    &mut schema_versions,
                )?;
                (version, input_bytes, input_sha256)
            }
            _ => unreachable!(),
        };
        let shard_summary = shard.finish();
        let manifest = args
            .manifest
            .get(index)
            .map(|path| quality_manifest_identity(path))
            .transpose()?;
        inputs.push(QualityInputIdentityV1 {
            path: path.display().to_string(),
            format: format.to_string(),
            input_bytes,
            input_sha256,
            decoded_bytes,
            decoded_sha256: shard_hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            schema_versions,
            pack_format_version,
            manifest,
        });
        if args.include_shards {
            shards.push(QualityShardV1 {
                input_index: index,
                summary: shard_summary,
            });
        }
    }

    let report = QualityReportV1 {
        quality_report_schema_version: QUALITY_REPORT_SCHEMA_VERSION,
        shogiesa_version: env!("CARGO_PKG_VERSION").to_string(),
        inputs,
        aggregate_decoded_sha256: aggregate_hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        summary: aggregate.finish(),
        shards,
    };
    let bytes = serde_json::to_vec_pretty(&report)?;
    atomic_write_bytes(&args.out, &[bytes.as_slice(), b"\n"].concat())?;
    eprintln!(
        "done: {} accepted, {} rejected across {} input(s) → {:?}",
        report.summary.accepted_records,
        report.summary.rejected_records,
        report.inputs.len(),
        args.out
    );
    Ok(())
}
