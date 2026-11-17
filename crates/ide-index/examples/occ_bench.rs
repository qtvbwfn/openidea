//! Occurrence-index benchmark over a real repository (issue #2024 follow-up).
//!
//! `cargo run --release --example occ_bench -- <project-dir>`
//!
//! Measures, against the given directory: cold index build (scan + extraction
//! + persistence), index DB size on disk, process RSS after the scan settles,
//! `usages_of` lookup latency over real indexed names, and a recall spot-check
//! of the inverted map against plain text search.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ide_core::{NullSink, SharedSink};
use ide_index::ProjectIndex;
use ide_language::{LanguageRegistry, Syntax};

fn main() {
    let Some(target) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: occ_bench <project-dir>");
        std::process::exit(2);
    };
    let target = dunce::canonicalize(&target).expect("project dir");
    let cache = std::env::temp_dir().join(format!("occ-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).unwrap();

    let pid = std::process::id();
    let rss_mib = || -> f64 {
        // macOS/Linux: `ps -o rss=` reports KiB.
        let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output();
        out.ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
    };

    let languages = Arc::new(LanguageRegistry::load(None).unwrap());
    let syntax = Arc::new(Syntax::new());
    let sink: SharedSink = Arc::new(NullSink);

    println!("target: {}", target.display());
    // Baseline: an empty project's index measures the fixed cost (language
    // registry, tree-sitter grammars, allocator warm-up), so the target's
    // RSS above it is the index's own footprint.
    let empty = std::env::temp_dir().join(format!("occ-bench-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    {
        let languages = Arc::new(LanguageRegistry::load(None).unwrap());
        let syntax = Arc::new(Syntax::new());
        let base = ProjectIndex::open(
            &empty,
            &empty,
            Vec::<PathBuf>::new(),
            vec![],
            languages,
            syntax,
            Arc::new(NullSink),
        )
        .unwrap();
        assert!(base.wait_idle(Duration::from_secs(60)));
    }
    println!("RSS baseline (empty project): {:.0} MiB", rss_mib());

    let started = Instant::now();
    let index = ProjectIndex::open(&target, &cache, Vec::<PathBuf>::new(), vec![], languages, syntax, sink)
        .expect("index open");
    let idle = index.wait_idle(Duration::from_secs(600));
    let build = started.elapsed();
    let status = index.status();
    println!(
        "cold build: {build:.2?} (wait_idle={idle}) — files {}, symbols {}",
        status.files, status.symbols
    );
    println!("RSS after build: {:.0} MiB", rss_mib());
    index.compact();
    println!("RSS after compact: {:.0} MiB", rss_mib());

    let mut db: u64 = 0;
    let mut files: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(&cache) {
        for e in entries.flatten() {
            let meta = e.metadata().unwrap();
            if meta.is_file() {
                files += 1;
                db += meta.len();
            }
        }
    }
    println!("index DB: {:.1} MiB across {files} file(s)", db as f64 / 1048576.0);

    // Pick real indexed names: sample symbols via find_symbols, plus a few misses.
    let mut names: BTreeSet<String> = BTreeSet::new();
    let probes = ["a", "e", "i", "o", "u", "s", "r", "t", "n", "l"];
    for q in probes {
        for m in index.find_symbols(q, ide_index::SymbolFilter::All, 40) {
            if names.len() >= 24 {
                break;
            }
            let name = m.name.clone();
            if name.len() >= 4 {
                names.insert(name);
            }
        }
    }
    let names: Vec<String> = names.into_iter().collect();
    println!("probing usages_of with {} real indexed names", names.len());

    let mut total_us = 0u128;
    let mut max_us = 0u128;
    let mut total_hits = 0usize;
    // Median-of-5 per name: the first lookup pays page-in and cache misses.
    for name in &names {
        let mut samples: Vec<u128> = Vec::with_capacity(5);
        for _ in 0..5 {
            let t = Instant::now();
            let hits = index.usages_of(name, false).len();
            samples.push(t.elapsed().as_micros());
            total_hits = total_hits.max(hits);
        }
        samples.sort_unstable();
        total_us += samples[2];
        max_us = max_us.max(samples[4]);
    }
    println!(
        "usages_of: avg {:.0} µs, max {max_us} µs over {} lookups ({} hits total)",
        total_us / names.len().max(1) as u128,
        names.len(),
        total_hits
    );

    // Recall spot-check, done the way #2024 defines it: sample real code files,
    // extract the identifier tokens they contain, and check how many of those
    // tokens the inverted map covers for that file (a token counts as covered
    // when `usages_of(token)` lists an occurrence in the same file). Names are
    // filtered to identifier-shaped probes (no kebab-case component tags).
    let mut names: Vec<String> = names
        .into_iter()
        .filter(|n| !n.contains('-') && n.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_'))
        .collect();

    // Sample up to 6 code files of mixed kinds from the target.
    let mut sample: Vec<PathBuf> = Vec::new();
    for ext in ["ts", "vue", "tsx"] {
        let mut found = index.files_with_extensions(&[ext.to_string()], 400);
        let step = (found.len() / 3).max(1);
        sample.extend(found.drain(..).step_by(step).take(2));
    }
    sample.retain(|p| p.is_file());
    println!("\nrecall spot-check over {} sampled files:", sample.len());
    let mut covered_total = 0usize;
    let mut token_total = 0usize;
    for file in &sample {
        let Ok(full) = std::fs::read_to_string(file) else { continue };
        // For .vue files only the <script> block is in the extraction's scope
        // (template identifiers are a different, larger feature); scan just that.
        let text: String = if file.extension().is_some_and(|e| e == "vue") {
            match full.find("<script") {
                Some(start) => {
                    let rest = &full[start..];
                    let end = rest.find("</script>").unwrap_or(rest.len());
                    rest[..end].to_string()
                }
                None => String::new(),
            }
        } else {
            full
        };
        let text_ref: &str = &text;
        let bytes = text_ref.as_bytes();
        let mut tokens: BTreeSet<String> = BTreeSet::new();
        let mut ident = String::new();
        let mut i = 0usize;
        let mut in_str: Option<u8> = None;
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        let mut prev = 0u8;
        while i < bytes.len() {
            let b = bytes[i];
            if in_line_comment {
                if b == b'\n' {
                    in_line_comment = false;
                }
            } else if in_block_comment {
                if prev == b'*' && b == b'/' {
                    in_block_comment = false;
                }
            } else if let Some(q) = in_str {
                if b == b'\\' {
                    i += 1;
                } else if b == q {
                    in_str = None;
                }
            } else if (b == b'"' || b == b'\'' || b == b'`') && prev != b'\\' {
                in_str = Some(b);
            } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_line_comment = true;
            } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                in_block_comment = true;
            } else if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
                ident.push(b as char);
            } else {
                if ident.len() >= 4
                    && !ident.chars().next().unwrap().is_ascii_digit()
                    && !matches!(
                        ident.as_str(),
                        "this" | "const" | "let" | "var" | "function" | "return" | "import" | "export"
                            | "from" | "class" | "interface" | "type" | "true" | "false" | "null"
                            | "undefined" | "if" | "else" | "for" | "while" | "new" | "await"
                            | "async" | "typeof" | "instanceof" | "default" | "extends" | "static"
                            | "throw" | "catch" | "try" | "switch" | "case" | "break" | "continue"
                            | "string" | "number" | "boolean" | "value" | "index" | "length"
                    )
                {
                    tokens.insert(ident.clone());
                }
                ident.clear();
            }
            prev = b;
            i += 1;
        }
        let mut covered = 0usize;
        for token in &tokens {
            let in_file = index
                .usages_of(token, false)
                .iter()
                .any(|loc| file.ends_with(Path::new(loc.path.as_os_str())));
            covered += usize::from(in_file);
        }
        covered_total += covered;
        token_total += tokens.len();
        println!(
            "  {}: {covered}/{} identifier tokens covered",
            file.file_name().unwrap_or_default().to_string_lossy(),
            tokens.len()
        );
    }
    if token_total > 0 {
        println!(
            "overall: {covered_total}/{token_total} = {:.1}% of sampled file identifiers covered by the inverted map",
            covered_total as f64 / token_total as f64 * 100.0
        );
    }

    println!("RSS at end: {:.0} MiB", rss_mib());

    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&empty);
}
