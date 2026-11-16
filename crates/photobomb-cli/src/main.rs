//! photobomb: command-line front end for photobomb-core.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use photobomb_core::{group, Group, Library, Op, SimilarOpts};

#[derive(Parser)]
#[command(
    name = "photobomb",
    version,
    about = "Find and clean up duplicate and similar photos"
)]
struct Cli {
    /// Where the index lives. Defaults to the same library the desktop app uses.
    #[arg(long, global = true, env = "PHOTOBOMB_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage the folders that get scanned
    Sources {
        #[command(subcommand)]
        cmd: SourceCmd,
    },
    /// Scan all sources (folders given here are added as sources first)
    Scan { dirs: Vec<PathBuf> },
    /// Show library totals
    Status,
    /// List groups of byte-identical files
    Exact {
        #[arg(long)]
        json: bool,
    },
    /// List groups of visually similar photos
    Similar {
        #[command(flatten)]
        opts: SimilarArgs,
        #[arg(long)]
        json: bool,
    },
    /// Keep the suggested photo in every group and act on the rest
    Clean {
        kind: Kind,
        #[command(flatten)]
        opts: SimilarArgs,
        #[command(flatten)]
        act: ActionArgs,
    },
    /// Act on specific photos by id (ids are shown by `exact` and `similar`)
    Apply {
        #[arg(long, value_delimiter = ',', required = true)]
        ids: Vec<i64>,
        #[command(flatten)]
        act: ActionArgs,
    },
    /// Mark photos as "not duplicates of each other"
    Ignore {
        #[arg(long, value_delimiter = ',', required = true)]
        ids: Vec<i64>,
        /// Only take this one photo out of the group; the others stay grouped
        #[arg(long)]
        only: Option<i64>,
    },
    /// List photos that are in no exact or similar group
    Others {
        #[command(flatten)]
        opts: SimilarArgs,
        #[arg(long)]
        json: bool,
    },
    /// Show everything known about one photo (ids are shown by `exact`, `similar`, `others`)
    Info {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// List files that could not be read as images
    Unreadable,
    /// List past operations
    History,
    /// Reverse a past operation
    Undo { batch: i64 },
}

#[derive(Subcommand)]
enum SourceCmd {
    List,
    Add {
        dir: PathBuf,
        /// The folder is on a removable drive: keep its files in the index when they cannot be found
        #[arg(long)]
        removable: bool,
    },
    Remove {
        dir: PathBuf,
    },
    /// Mark a source as being on a removable drive (on) or not (off)
    Removable {
        dir: PathBuf,
        state: OnOff,
    },
}

#[derive(Clone, Copy, ValueEnum, PartialEq)]
enum OnOff {
    On,
    Off,
}

fn find_source(lib: &Library, dir: &Path) -> Result<photobomb_core::Source> {
    // The folder may be unplugged, so fall back to the path as typed.
    let wanted = dunce::canonicalize(dir)
        .unwrap_or(absolute(dir)?)
        .to_string_lossy()
        .to_string();
    let wanted = wanted.trim_end_matches(['/', '\\']).to_string();
    match lib.sources()?.into_iter().find(|s| s.root == wanted) {
        Some(s) => Ok(s),
        None => bail!("{wanted} is not a source"),
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Exact,
    Similar,
}

#[derive(Clone, Copy, ValueEnum)]
enum Action {
    Trash,
    Move,
    Hardlink,
}

#[derive(Args)]
struct SimilarArgs {
    /// 0 = identical image, 4 = strict, 8 = balanced, 12 = loose, 16 = very loose
    #[arg(long, default_value_t = 8)]
    threshold: u32,
    /// Only group photos with the same shape
    #[arg(long)]
    same_aspect: bool,
    /// Only group photos taken within this many seconds of each other
    #[arg(long)]
    time_window: Option<i64>,
}

impl SimilarArgs {
    fn opts(&self) -> SimilarOpts {
        SimilarOpts {
            threshold: self.threshold,
            same_aspect: self.same_aspect,
            time_window_secs: self.time_window,
        }
    }
}

#[derive(Args)]
struct ActionArgs {
    /// What to do with the selected files
    #[arg(long, value_enum, default_value = "trash")]
    action: Action,
    /// Destination folder for --action move
    #[arg(long)]
    dest: Option<PathBuf>,
    /// Actually do it. Without this flag the command only prints what would happen.
    #[arg(long)]
    yes: bool,
    /// Allow removing every copy in a group
    #[arg(long)]
    allow_empty_groups: bool,
}

impl ActionArgs {
    fn op(&self) -> Result<Op> {
        Ok(match self.action {
            Action::Trash => Op::Trash,
            Action::Hardlink => Op::Hardlink,
            Action::Move => match &self.dest {
                Some(d) => Op::Move {
                    dest: absolute(d)?.to_string_lossy().to_string(),
                },
                None => bail!("--action move needs --dest <folder>"),
            },
        })
    }
}

fn absolute(p: &Path) -> Result<PathBuf> {
    Ok(if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    })
}

fn human(bytes: i64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", units[u])
    }
}

fn print_groups(groups: &[Group]) {
    for (n, g) in groups.iter().enumerate() {
        println!(
            "\nGroup {} — {} files, {} reclaimable",
            n + 1,
            g.members.len(),
            human(g.reclaimable)
        );
        for m in &g.members {
            let p = &m.photo;
            let dims = match (p.width, p.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => "-".into(),
            };
            let mut tags = Vec::new();
            if p.id == g.keeper {
                tags.push("keep".to_string());
            }
            if let (Some(d), true) = (m.distance, p.id != g.keeper) {
                tags.push(format!("distance {d}"));
            }
            if m.exact_copy {
                tags.push("identical bytes".into());
            }
            if m.hardlinked {
                tags.push("hard link".into());
            }
            if p.missing {
                tags.push("offline".into());
            }
            println!(
                "  [{:>6}] {:>10} {:>11}  {}{}",
                p.id,
                human(p.size),
                dims,
                p.path,
                if tags.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", tags.join(", "))
                }
            );
        }
    }
    let photos: usize = groups.iter().map(|g| g.members.len()).sum();
    let bytes: i64 = groups.iter().map(|g| g.reclaimable).sum();
    println!(
        "\n{} groups, {} files, {} reclaimable",
        groups.len(),
        photos,
        human(bytes)
    );
}

fn run_action(
    lib: &Library,
    ids: &[i64],
    act: &ActionArgs,
    similar: Option<&SimilarOpts>,
) -> Result<()> {
    let op = act.op()?;
    let plan = lib.plan(ids, &op, similar, act.allow_empty_groups)?;
    for item in &plan.items {
        let detail = match (&item.note, &item.target) {
            (Some(n), _) => format!("  — {n}"),
            (None, Some(t)) => format!("  -> {t}"),
            _ => String::new(),
        };
        println!("  {:<7} {}{}", item.status, item.path, detail);
    }
    println!(
        "\n{} to process ({}), {} skipped, {} blocked",
        plan.ok,
        human(plan.bytes),
        plan.skipped,
        plan.blocked
    );
    if !act.yes {
        println!("Dry run. Add --yes to apply.");
        return Ok(());
    }
    let res = lib.execute(ids, &op, similar, act.allow_empty_groups)?;
    println!(
        "Batch {}: {} done ({}), {} failed, {} skipped. Undo with: photobomb undo {}",
        res.batch_id,
        res.done,
        human(res.bytes),
        res.failed,
        res.skipped,
        res.batch_id
    );
    for e in &res.errors {
        eprintln!("  error: {e}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let lib = match &cli.data_dir {
        Some(d) => Library::open(d)?,
        None => Library::open_default()?,
    };

    match cli.cmd {
        Cmd::Sources { cmd } => match cmd {
            SourceCmd::List => {
                for s in lib.sources()? {
                    println!(
                        "[{}] {}  ({} files{}{}{})",
                        s.id,
                        s.root,
                        s.photo_count,
                        if s.missing_count > 0 {
                            format!(", {} not found", s.missing_count)
                        } else {
                            String::new()
                        },
                        if s.removable { ", removable" } else { "" },
                        if s.online { "" } else { ", OFFLINE" }
                    );
                }
            }
            SourceCmd::Add { dir, removable } => {
                let s = lib.add_source(&dir)?;
                if removable {
                    lib.set_removable(s.id, true)?;
                }
                println!(
                    "Added {}{}",
                    s.root,
                    if removable { " (removable)" } else { "" }
                );
            }
            SourceCmd::Remove { dir } => {
                let s = find_source(&lib, &dir)?;
                lib.remove_source(s.id)?;
                println!("Removed {} (files on disk untouched)", s.root);
            }
            SourceCmd::Removable { dir, state } => {
                let s = find_source(&lib, &dir)?;
                lib.set_removable(s.id, state == OnOff::On)?;
                println!(
                    "{} is {}",
                    s.root,
                    if state == OnOff::On {
                        "removable: files that cannot be found stay in the index"
                    } else {
                        "not removable: files that are gone are dropped at the next scan"
                    }
                );
            }
        },
        Cmd::Scan { dirs } => {
            for d in &dirs {
                lib.add_source(d)?;
            }
            if lib.sources()?.is_empty() {
                bail!("no sources. Try: photobomb scan <folder>");
            }
            let progress = |p: photobomb_core::ScanProgress| {
                if p.total > 0 {
                    eprint!("\r\x1b[K{}: {}/{}", p.stage, p.done, p.total);
                } else {
                    eprint!("\r\x1b[K{}: {}", p.stage, p.done);
                }
            };
            let s = photobomb_core::scan::scan(&lib, &progress, &AtomicBool::new(false))?;
            eprintln!("\r\x1b[K");
            println!(
                "Scanned {} files in {:.1}s: {} new, {} changed, {} gone, {} analysed, {} unreadable, {} similar pairs",
                s.files_seen, s.seconds, s.added, s.changed, s.removed, s.decoded, s.decode_failed, s.pairs
            );
            if s.missing > 0 {
                println!(
                    "{} file(s) not found but kept (offline or removable source)",
                    s.missing
                );
            }
            for o in &s.offline_sources {
                println!("Skipped offline source: {o}");
            }
            let exact = lib.exact_groups()?;
            let similar = lib.similar_groups(&SimilarOpts::default())?;
            println!(
                "{} exact groups ({} reclaimable), {} similar groups at the default level",
                exact.len(),
                human(exact.iter().map(|g| g.reclaimable).sum()),
                similar.len()
            );
        }
        Cmd::Status => {
            let s = lib.status()?;
            println!("Library:    {}", lib.data_dir.display());
            println!("Sources:    {}", s.sources);
            println!("Files:      {} ({})", s.photos, human(s.bytes));
            println!("Analysed:   {}", s.decoded);
            println!("Unreadable: {}", s.unreadable);
            println!("Pairs:      {}", s.pairs);
            println!(
                "Not found:  {} (kept: offline or removable source)",
                s.missing
            );
            println!("Ignored:    {} decision(s)", lib.ignored_count()?);
        }
        Cmd::Exact { json } => {
            let groups = lib.exact_groups()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
            } else {
                print_groups(&groups);
            }
        }
        Cmd::Similar { opts, json } => {
            let groups = lib.similar_groups(&opts.opts())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
            } else {
                print_groups(&groups);
            }
        }
        Cmd::Clean { kind, opts, act } => {
            let sopts = opts.opts();
            let (groups, similar) = match kind {
                Kind::Exact => (lib.exact_groups()?, None),
                Kind::Similar => (lib.similar_groups(&sopts)?, Some(&sopts)),
            };
            let ids: Vec<i64> = group::auto_select(&groups).iter().map(|p| p.id).collect();
            if ids.is_empty() {
                println!("Nothing to clean.");
                return Ok(());
            }
            run_action(&lib, &ids, &act, similar)?;
        }
        Cmd::Apply { ids, act } => run_action(&lib, &ids, &act, None)?,
        Cmd::Ignore {
            ids,
            only: Some(id),
        } => {
            lib.ignore_photo(id, &ids)?;
            println!("Ignored. Photo {id} will no longer be grouped with the others.");
        }
        Cmd::Ignore { ids, only: None } => {
            lib.ignore_group(&ids)?;
            println!("Ignored. These photos will no longer be grouped together.");
        }
        Cmd::Others { opts, json } => {
            let photos = lib.other_photos(&opts.opts())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&photos)?);
            } else {
                for p in &photos {
                    let dims = match (p.width, p.height) {
                        (Some(w), Some(h)) => format!("{w}x{h}"),
                        _ => "-".into(),
                    };
                    let mut tags = Vec::new();
                    if p.missing {
                        tags.push("offline");
                    }
                    if p.not_dup {
                        tags.push("marked not a duplicate");
                    }
                    println!(
                        "  [{:>6}] {:>10} {:>11}  {}{}",
                        p.id,
                        human(p.size),
                        dims,
                        p.path,
                        if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  ({})", tags.join(", "))
                        }
                    );
                }
                println!(
                    "\n{} files in no group, {}",
                    photos.len(),
                    human(photos.iter().map(|p| p.size).sum())
                );
            }
        }
        Cmd::Info { id, json } => {
            let d = lib.photo_details(id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&d)?);
            } else {
                let p = &d.photo;
                let opt = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
                println!("Path:         {}", p.path);
                println!(
                    "Source:       {}{}",
                    d.source_root,
                    if d.source_removable {
                        " (removable)"
                    } else {
                        ""
                    }
                );
                println!("On disk now:  {}", if d.exists { "yes" } else { "no" });
                println!(
                    "State:        {}{}",
                    d.state,
                    if p.missing {
                        ", not found at last scan"
                    } else {
                        ""
                    }
                );
                println!("Size:         {} ({} bytes)", human(p.size), p.size);
                println!("Format:       {}", opt(&p.format));
                match (p.width, p.height) {
                    (Some(w), Some(h)) => println!("Dimensions:   {w} x {h}"),
                    _ => println!("Dimensions:   -"),
                }
                println!(
                    "Taken (EXIF): {}",
                    p.taken_at
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".into())
                );
                println!("Modified ns:  {}", p.mtime_ns);
                println!("Device/inode: {}/{}", p.dev, p.ino);
                println!("Content hash: {}", opt(&p.content_hash));
                println!("Partial hash: {}", opt(&d.partial_hash));
                println!("pHash/dHash:  {} / {}", opt(&d.phash), opt(&d.dhash));
                println!(
                    "Not a dup:    {}",
                    if p.not_dup { "yes (flagged)" } else { "no" }
                );
                if let Some(e) = &p.decode_error {
                    println!("Decode error: {e}");
                }
                if !d.exif.is_empty() {
                    println!("EXIF:");
                    for (k, v) in &d.exif {
                        println!("  {k}: {v}");
                    }
                }
            }
        }
        Cmd::Unreadable => {
            for p in lib.unreadable()? {
                println!("{}  — {}", p.path, p.decode_error.unwrap_or_default());
            }
        }
        Cmd::History => {
            for h in lib.history()? {
                println!(
                    "[{}] {:<8} {} files, {} done, {} failed, {} undone, {}",
                    h.batch_id,
                    h.op,
                    h.files,
                    h.done,
                    h.failed,
                    h.undone,
                    human(h.bytes)
                );
            }
        }
        Cmd::Undo { batch } => {
            let r = lib.undo(batch)?;
            println!("Restored {} file(s), {} failed", r.done, r.failed);
            for e in &r.errors {
                eprintln!("  error: {e}");
            }
        }
    }
    Ok(())
}
