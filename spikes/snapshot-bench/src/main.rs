//! CLI for the Gate 2 snapshot benchmarks. Non-production.

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use eyre::{Context, Result};

use snapshot_bench::chain::{self, SeedManifest};
use snapshot_bench::snapshot::{self, Limits};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Deploy a fresh Classic GTCR via the real factory on an anvil fork and seed it
    /// with N synthetic V1-schema entries.
    Seed {
        #[arg(long)]
        rpc_url: String,
        #[arg(long)]
        items: u64,
        /// Where to write the seed manifest (trusted-anchor record for this bench).
        #[arg(long)]
        out: PathBuf,
    },
    /// Harvest proofs at the manifest anchor and emit the snapshot + metrics.
    Generate {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Fully verify a snapshot (raw JSON or gzip transport form) against the
    /// manifest's pinned verifier profile: version, chain, registry, anchor block
    /// number/hash/state root, and the PROVEN runtime codehash must all match.
    Verify {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
    },
    /// Emit the profile-binding ATTACK artifact: an honest empty-catalog snapshot for
    /// the funded seeder EOA under the same anchor root (fixture for rejection tests).
    EoaProbe {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Install-path bench (non-production prototype): build a synthetic skill tree,
    /// CAR it, then run the consumer path — preflight-validate the complete DAG
    /// against the EXPECTED Tree CID, stage, atomically publish, byte-compare, and
    /// write a lockfile.
    CarBench {
        /// Number of synthetic files in the tree.
        #[arg(long)]
        files: u64,
        /// Bytes per file.
        #[arg(long)]
        file_bytes: u64,
        /// Working directory (source/, tree.car, out/, lockfile.json under it).
        #[arg(long)]
        work: PathBuf,
        /// Expected Tree CID (canonical text form). Defaults to the CID the builder
        /// just produced; pass a different one to watch the install refuse it.
        #[arg(long)]
        expected_cid: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Args::parse().cmd {
        Cmd::Seed {
            rpc_url,
            items,
            out,
        } => {
            let manifest = chain::seed(&rpc_url, items).await?;
            std::fs::write(&out, serde_json::to_string_pretty(&manifest)?)
                .wrap_err_with(|| format!("writing {}", out.display()))?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Cmd::Generate { manifest, out } => {
            let manifest: SeedManifest = read_json(&manifest)?;
            let (snapshot, stats) = chain::generate(&manifest).await?;
            std::fs::write(&out, serde_json::to_vec(&snapshot)?)
                .wrap_err_with(|| format!("writing {}", out.display()))?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Cmd::Verify {
            manifest,
            snapshot: snapshot_path,
        } => {
            let manifest: SeedManifest = read_json(&manifest)?;
            let limits = Limits::default();
            let started = Instant::now();
            // File-boundary reader: caps are enforced at the read boundary
            // (at most cap + 1 bytes are ever read / logically retained per
            // buffer; heap capacity may carry allocator growth overhead).
            let (parsed, decoded_bytes) =
                snapshot::read_snapshot_file_bounded(&snapshot_path, &limits)?;
            let parse_secs = started.elapsed().as_secs_f64();
            let verify_started = Instant::now();
            let stats = snapshot::verify(&parsed, &manifest.profile(), &limits)?;
            let verify_secs = verify_started.elapsed().as_secs_f64();
            // Spec §6 step 7 structural screening (offline-checkable subset): the
            // decoded six columns must pass Reserved-empty + canonical-Tree-CID.
            let screen_started = Instant::now();
            for (i, row) in parsed.rows.iter().enumerate() {
                let d = snapshot_bench::schema::Descriptor::decode(&row.descriptor)
                    .map_err(|e| eyre::eyre!("row {i}: {e}"))?;
                snapshot_bench::schema::screen(&d)
                    .map_err(|e| eyre::eyre!("row {i} fails policy screening: {e}"))?;
            }
            let screen_secs = screen_started.elapsed().as_secs_f64();
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "items": stats.items,
                    "slotProofsChecked": stats.slot_proofs_checked,
                    "uniqueNodes": stats.unique_nodes,
                    "storageRoot": stats.storage_root,
                    "decodedBytes": decoded_bytes,
                    "parseSecs": parse_secs,
                    "verifySecs": verify_secs,
                    "screenSecs": screen_secs,
                    "totalSecs": parse_secs + verify_secs + screen_secs,
                })
            );
        }
        Cmd::EoaProbe { manifest, out } => {
            let manifest: SeedManifest = read_json(&manifest)?;
            let snapshot = chain::eoa_probe(&manifest).await?;
            std::fs::write(&out, serde_json::to_vec(&snapshot)?)
                .wrap_err_with(|| format!("writing {}", out.display()))?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "registry": snapshot.binding.registry,
                    "itemCount": snapshot.item_count,
                    "note": "attack artifact — must FAIL verification under the pinned profile",
                })
            );
        }
        Cmd::CarBench {
            files,
            file_bytes,
            work,
            expected_cid,
        } => {
            use snapshot_bench::car;
            use std::collections::BTreeMap;
            let source = work.join("source");
            let out = work.join("out");
            let _ = std::fs::remove_dir_all(&source);
            let _ = std::fs::remove_dir_all(&out);
            std::fs::create_dir_all(source.join("references"))?;
            // Deterministic synthetic skill-shaped tree: SKILL.md + reference files.
            std::fs::write(
                source.join("SKILL.md"),
                "---\nname: bench-skill\ndescription: install-path benchmark tree\n---\nbody\n",
            )?;
            for i in 0..files {
                let mut data = Vec::with_capacity(file_bytes as usize);
                while data.len() < file_bytes as usize {
                    data.extend_from_slice(format!("ref {i} block {} \n", data.len()).as_bytes());
                }
                data.truncate(file_bytes as usize);
                std::fs::write(
                    source.join("references").join(format!("ref-{i:03}.md")),
                    data,
                )?;
            }

            let build_started = Instant::now();
            let mut blocks = BTreeMap::new();
            let (built_root, _tsize) = car::build_dir(&source, &mut blocks)?;
            let car_bytes = car::write_car(built_root, &blocks);
            let build_secs = build_started.elapsed().as_secs_f64();
            std::fs::write(work.join("tree.car"), &car_bytes)?;

            // The EXPECTED Tree CID is an input to the consumer path (in production
            // it comes from the verified descriptor). Defaulting to the builder's CID
            // keeps the bench self-contained; the CAR's own root claim is never the
            // authority — `install` checks against `expected`.
            let expected = match expected_cid {
                Some(s) => car::Cid::parse_canonical(&s)?,
                None => built_root,
            };
            let verify_started = Instant::now();
            let (_claimed_root, read_blocks) = car::read_car(&car_bytes)?;
            let plan = car::install(expected, &read_blocks, &out)?;
            let verify_install_secs = verify_started.elapsed().as_secs_f64();

            // Byte-for-byte comparison against the source tree.
            let installed = plan.locked_files();
            for f in &installed {
                let a = std::fs::read(source.join(&f.path))?;
                let b = std::fs::read(out.join(&f.path))?;
                eyre::ensure!(a == b, "byte mismatch at {}", f.path);
            }

            let lockfile = car::Lockfile {
                spec: car::LOCKFILE_SPEC.into(),
                tree_cid: expected.to_string_canonical(),
                root_block_sha256: car::hex_lower(&expected.digest),
                blocks: plan.blocks,
                total_bytes: plan.total_bytes,
                files: installed,
                // Registry identity is the production CLI's scope; the bench records
                // its absence honestly instead of placeholder values.
                identity: None,
            };
            std::fs::write(
                work.join("lockfile.json"),
                serde_json::to_string_pretty(&lockfile)?,
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "treeCid": expected.to_string_canonical(),
                    "files": files + 1,
                    "sourceBytes": files * file_bytes,
                    "carBytes": car_bytes.len(),
                    "blocks": plan.blocks,
                    "materializedBytes": plan.total_bytes,
                    "buildSecs": build_secs,
                    "verifyInstallSecs": verify_install_secs,
                })
            );
        }
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T> {
    let raw = std::fs::read(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&raw).wrap_err_with(|| format!("parsing {}", path.display()))
}
