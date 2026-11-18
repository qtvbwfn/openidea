use super::doc::blocking;
use crate::Host;
use crate::Project;
use ide_command::command;
use ide_core::paths::{AliasMap, resolve_spec};
use ide_core::{CancellationToken, DocId, IdeResult, IpcPath, LanguageId, Location, Pos, SymbolKind};
use ide_index::{ExtensionCount, FileMatch, IndexStatus, ProjectIndex, SymbolFilter, SymbolMatch};
use ide_language::{FileReference, ImportName, WebReference};
use ide_syntax::{Locals, Snapshot as StructureSnapshot};
use ide_workspace::file_refs::{self, PathCompletion};
use ide_workspace::roots::alias_map;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A web reference no language server resolves and where it points: `class="card"` →
/// `.card` selectors, `id` → `#id` selectors, `href="#top"` → the element, `var(--x)` →
/// `--x:` declarations.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WebGoto {
    pub reference: WebReference,
    pub targets: Vec<Location>,
}

const MAX_WEB_TARGETS: usize = 200;

/// A Python class definition and the base names written in its source. The UI builds a
/// name → locations index from every class in the project (attached dependency roots
/// included) to compute ↑ overrides / ↓ overridden gutter markers without a language
/// server's type-hierarchy provider.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClassInheritance {
    /// Start of the class definition's selection range (0-based line, UTF-16 column).
    pub path: IpcPath,
    pub line: u32,
    pub col: u32,
    pub name: String,
    /// Base names as written in source (bare name, e.g. `AllToAllTokenDispatcher`; a
    /// qualified name keeps only its last segment).
    pub bases: Vec<String>,
}

/// The project root and its resource roots (served at `/`), for file references.
fn reference_roots(project: &Project) -> (PathBuf, Vec<PathBuf>) {
    let resources = project.roots().resource_dirs().iter().map(|d| project.root.join(d)).collect();
    (project.root.clone(), resources)
}

#[command(trust = always)]
pub fn index_status(host: &Host, window: &str) -> IdeResult<IndexStatus> {
    Ok(host.project(window)?.index.status())
}

#[command(trust = always)]
pub fn index_file_extensions(host: &Host, window: &str) -> IdeResult<Vec<ExtensionCount>> {
    // Most common first.
    Ok(host.project(window)?.index.file_extensions())
}

#[command(trust = always)]
pub async fn find_files(host: &Host, window: &str, query: String, limit: u32) -> IdeResult<Vec<FileMatch>> {
    let project = host.project(window)?;
    blocking(move || Ok(project.index.find_files(&query, limit as usize))).await
}

#[command(trust = always)]
pub async fn find_files_named(host: &Host, window: &str, name: String) -> IdeResult<Vec<IpcPath>> {
    let project = host.project(window)?;
    blocking(move || Ok(project.index.files_named(&name).into_iter().map(Into::into).collect())).await
}

/// Folders holding indexed files, for `@` references.
#[command(trust = always)]
pub async fn find_dirs(host: &Host, window: &str, query: String, limit: u32) -> IdeResult<Vec<FileMatch>> {
    let project = host.project(window)?;
    blocking(move || Ok(project.index.find_dirs(&query, limit as usize))).await
}

#[command(trust = always)]
pub async fn find_symbols(
    host: &Host,
    window: &str,
    query: String,
    filter: SymbolFilter,
    limit: u32,
) -> IdeResult<Vec<SymbolMatch>> {
    let project = host.project(window)?;
    blocking(move || Ok(project.index.find_symbols(&query, filter, limit as usize))).await
}

/// Index symbols named exactly one of `names` inside `within` (an agent task's folder).
#[command(trust = always)]
pub async fn find_symbols_named(
    host: &Host,
    window: &str,
    names: Vec<String>,
    within: IpcPath,
) -> IdeResult<Vec<SymbolMatch>> {
    // At most `SYMBOLS_PER_NAME` each: every name an agent reply shows, in one lookup.
    const SYMBOLS_PER_NAME: usize = 50;
    let project = host.project(window)?;
    blocking(move || Ok(project.index.symbols_named_any(&names, &PathBuf::from(within), SYMBOLS_PER_NAME))).await
}

#[command(trust = always)]
pub fn mark_recent(host: &Host, window: &str, path: PathBuf) -> IdeResult<()> {
    host.project(window)?.index.mark_recent(&path);
    Ok(())
}

#[command(trust = always)]
pub fn remove_recent(host: &Host, window: &str, path: PathBuf) -> IdeResult<()> {
    host.project(window)?.index.remove_recent(&path);
    Ok(())
}

/// Recently opened files, most recent first (⌘E).
#[command(trust = always)]
pub fn recent_files(host: &Host, window: &str) -> IdeResult<Vec<ide_core::IpcPath>> {
    Ok(host.project(window)?.index.recent_files().into_iter().map(Into::into).collect())
}

/// The web reference at `pos` of `doc` and its targets; `None` when there is no reference.
#[command(trust = always)]
pub async fn web_goto(host: &Host, window: &str, doc: DocId, pos: Pos) -> IdeResult<Option<WebGoto>> {
    let project = host.project(window)?;
    let syntax = host.syntax();
    blocking(move || {
        let snap = project.documents.snapshot(doc)?;
        if snap.large {
            return Ok(None);
        }
        let Some(reference) = syntax.web_reference_at(&snap.language, &snap.text, pos) else { return Ok(None) };
        let here = |range| Location { path: snap.path.clone(), range, uri: None };
        let targets = match reference.symbol() {
            None => syntax.web_anchors(&snap.language, &snap.text, &reference.name).into_iter().map(here).collect(),
            Some((name, kind)) => {
                // The open document from its (possibly unsaved) text, other files from the index.
                let mut targets: Vec<Location> = syntax
                    .web_definitions(&snap.language, &snap.text)
                    .into_iter()
                    .filter(|t| t.name == name && t.kind == kind)
                    .map(|t| here(t.range))
                    .collect();
                targets.extend(
                    project
                        .index
                        .symbols_named(&name, Some(kind), MAX_WEB_TARGETS)
                        .into_iter()
                        .filter(|m| *m.path != *snap.path)
                        .map(|m| Location { path: m.path, range: m.range, uri: None }),
                );
                targets
            }
        };
        Ok(Some(WebGoto { reference, targets }))
    })
    .await
}

/// The file a Python import path (`a.b.c`, or `.x` relative to `from`) names; `None` when
/// no project or content root has it.
#[command(trust = always)]
pub async fn nav_resolve_module(host: &Host, window: &str, name: String, from: IpcPath) -> IdeResult<Option<IpcPath>> {
    // `a.b.c` is `a/b/c.py(i)` or `a/b/c/__init__.py(i)` in the project or one of its content roots
    // (an attached dependency project included). Neither the language server (it analyses a single
    // root) nor the definition index resolves a package segment, so ⌘B on the `a.b` of
    // `from a.b import x` needs this.
    let project = host.project(window)?;
    blocking(move || {
        let dots = name.len() - name.trim_start_matches('.').len();
        let parts: Vec<&str> = name.split('.').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Ok(None);
        }
        let rel = parts.iter().collect::<PathBuf>();
        let bases: Vec<PathBuf> = if dots > 0 {
            let dir = PathBuf::from(from);
            let dir = dir.parent().map(Path::to_path_buf).unwrap_or(dir);
            let base = (0..dots.saturating_sub(1)).fold(dir, |p, _| p.parent().map(Path::to_path_buf).unwrap_or(p));
            vec![base]
        } else {
            crate::project::navigation_roots(&project, &from)
        };
        for base in bases {
            let dir = base.join(&rel);
            for candidate in
                [dir.with_extension("py"), dir.with_extension("pyi"), dir.join("__init__.py"), dir.join("__init__.pyi")]
            {
                if candidate.is_file() {
                    return Ok(Some(IpcPath::from(candidate)));
                }
            }
        }
        Ok(None)
    })
    .await
}

/// Index definitions named `name` within the roots `from` may navigate into (its own root
/// plus its configured dependencies).
#[command(trust = always)]
pub async fn nav_index_definitions(
    host: &Host,
    window: &str,
    name: String,
    from: IpcPath,
) -> IdeResult<Vec<SymbolMatch>> {
    // The cross-root answer when the language server cannot resolve an import into an attached
    // dependency project. `from`'s own root is left out: the server sees it, so a name it could
    // not resolve there is not answered by every declaration that happens to share it (any `save`
    // for `repo.save()`).
    let project = host.project(window)?;
    blocking(move || {
        let roots = crate::project::navigation_roots(&project, &from);
        let Some((_own, roots)) = roots.split_first() else { return Ok(Vec::new()) };
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        // Filter inside the index: a cap applied before the root filter can fill with
        // own-root matches and evict the dependency roots entirely.
        Ok(project.index.symbols_named_within(&name, None, roots, 500))
    })
    .await
}

/// Every Python class in the project (attached dependency roots included) and its base names,
/// for native inheritance gutter markers.
#[command(trust = always)]
pub async fn nav_inheritance(host: &Host, window: &str) -> IdeResult<Vec<ClassInheritance>> {
    // The index covers content roots, so this is the whole-project answer when no type-hierarchy
    // provider exists.
    let project = host.project(window)?;
    let syntax = host.syntax();
    blocking(move || {
        let python = LanguageId::new("python");
        let mut cache = CLASS_BASES.lock();
        // Disk-backed classes come from the index: it already parsed every file once for
        // symbols, so no second parse of the project is needed here.
        let mut out: Vec<ClassInheritance> = project
            .index
            .inheritance()
            .into_iter()
            // The index holds classes of every language; the graph is Python's.
            .filter(|c| matches!(c.path.extension().and_then(std::ffi::OsStr::to_str), Some("py" | "pyi")))
            .map(|c| ClassInheritance {
                path: IpcPath::from(c.path.clone()),
                line: c.range.start.line,
                col: c.range.start.col,
                name: c.name.clone(),
                bases: c.bases.clone(),
            })
            .collect();
        // Open buffers may be unsaved, so their classes are parsed live (and cached per
        // text/stamp) and replace the index's view of that file. This keeps #3359's
        // "open buffer by its text" behaviour while the disk path is index-backed.
        let open_files: Vec<PathBuf> = project
            .index
            .files_with_extensions(&["py".into(), "pyi".into()], 20_000)
            .into_iter()
            .filter(|path| project.documents.text_of(path).is_some())
            .collect();
        for path in open_files {
            let open = project.documents.text_of(&path);
            let stamp = match &open {
                Some(text) => Stamp(text_hash(text)),
                None => continue,
            };
            let text = open.unwrap();
            let classes = match cache.get(&path) {
                Some((cached, classes)) if *cached == stamp => classes,
                _ => &cache.entry(path.clone()).insert_entry((stamp, syntax.class_bases(&python, &text))).into_mut().1,
            };
            out.retain(|c| c.path.0 != *path);
            out.extend(classes.iter().map(|class| ClassInheritance {
                path: IpcPath::from(path.clone()),
                line: class.line,
                col: class.col,
                name: class.name.clone(),
                bases: class.bases.clone(),
            }));
        }
        Ok(out)
    })
    .await
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp(u64);

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

type ClassBasesCache = std::collections::HashMap<PathBuf, (Stamp, Vec<ide_language::ClassBases>)>;

/// Class bases per Python file, reused while the file is unchanged.
static CLASS_BASES: std::sync::LazyLock<parking_lot::Mutex<ClassBasesCache>> =
    std::sync::LazyLock::new(Default::default);

/// The existing file or directory a path written in `from` names (`href`, `url()`, a string literal).
#[command(trust = always)]
pub async fn fs_resolve_reference(
    host: &Host,
    window: &str,
    from: IpcPath,
    value: String,
) -> IdeResult<Option<IpcPath>> {
    let project = host.project(window).ok();
    blocking(move || {
        let (root, resources) = project.as_deref().map(reference_roots).unzip();
        let resources = resources.unwrap_or_default();
        Ok(file_refs::resolve(&value, &from, root.as_deref(), &resources).map(Into::into))
    })
    .await
}

/// Files and folders that continue `prefix`, a path being typed in `from`.
#[command(trust = always)]
pub async fn path_completions(
    host: &Host,
    window: &str,
    from: IpcPath,
    prefix: String,
) -> IdeResult<Vec<PathCompletion>> {
    let project = host.project(window).ok();
    blocking(move || {
        let (root, resources) = project.as_deref().map(reference_roots).unzip();
        let resources = resources.unwrap_or_default();
        Ok(file_refs::complete(&prefix, &from, root.as_deref(), &resources))
    })
    .await
}

/// Paths in an HTML document or stylesheet that name no existing file.
#[command(trust = always)]
pub async fn web_unresolved_references(host: &Host, window: &str, doc: DocId) -> IdeResult<Vec<FileReference>> {
    let project = host.project(window)?;
    let syntax = host.syntax();
    blocking(move || {
        let snap = project.documents.snapshot(doc)?;
        if snap.large {
            return Ok(Vec::new());
        }
        let (root, resources) = reference_roots(&project);
        Ok(syntax
            .web_file_references(&snap.language, &snap.text)
            .into_iter()
            .filter(|r| {
                file_refs::is_file_reference(&r.value)
                    && !file_refs::path_part(&r.value).is_empty()
                    && file_refs::resolve(&r.value, &snap.path, Some(&root), &resources).is_none()
            })
            .collect())
    })
    .await
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexGoto {
    pub targets: Vec<Location>,
    /// `certain`: the import chain names the target. `candidate`: a name
    /// match (the component registry) — the UI labels it.
    pub note: IndexNote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum IndexNote {
    Certain,
    Candidate,
}

#[command(trust = always)]
pub async fn index_goto(host: &Host, window: &str, doc: DocId, pos: Pos, word: String) -> IdeResult<Option<IndexGoto>> {
    let project = host.project(window)?;
    let syntax = host.syntax();
    let structure = project.structure.clone();
    let documents = project.documents.clone();
    let index = project.index.clone();
    let root = project.root.clone();
    blocking(move || {
        let Ok(info) = documents.snapshot(doc) else { return Ok(None) };
        if info.large || word.is_empty() {
            return Ok(None);
        }
        let lang = info.language.clone();
        let text = info.text;
        let from: PathBuf = info.path.into();
        let mut word = word;

        // A local of a <script> region resolves in-file: the structure service
        // cannot see regions (a .vue file's grammar is HTML).
        if is_web_language(lang.as_str())
            && let Some((region, region_pos)) = syntax.script_region_at(&lang, &text, pos)
        {
            let snap = StructureSnapshot::text(&region.language, &region.text);
            let cancel = CancellationToken::new();
            if let Ok(locals) = structure.locals(&snap, region_pos, &cancel) {
                match locals {
                    Locals::NoIdentifier => return Ok(None),
                    Locals::Local { symbol } => {
                        return Ok(Some(IndexGoto {
                            targets: vec![Location {
                                path: from.into(),
                                range: region.shift(symbol.definition),
                                uri: None,
                            }],
                            note: IndexNote::Certain,
                        }));
                    }
                    Locals::NotLocal { name, .. } => word = name,
                }
            }
        }

        // The import chain: the word bound by an import names its module's export.
        let facts = syntax.script_facts(&lang, &text);
        let aliases = alias_map(&root, &from);
        let exists = |p: &Path| index.has_file(p);
        for import in &facts.imports {
            // "" names the default export, "*" a namespace import (the module file).
            let target_name = import.names.iter().find_map(|n| match n {
                ImportName::Named { imported, local } if local == &word => Some(imported.clone()),
                ImportName::Default { local } if local == &word => Some(String::new()),
                ImportName::Namespace { local } if local == &word => Some("*".to_string()),
                _ => None,
            });
            let Some(target_name) = target_name else { continue };
            let Some(target) = resolve_spec(&from, &import.source, &aliases, &exists) else { continue };
            if target_name == "*" {
                return Ok(Some(IndexGoto { targets: vec![file_location(&index, &target)], note: IndexNote::Certain }));
            }
            let mut visited = HashSet::new();
            if let Some(loc) = resolve_export(&index, &target, &target_name, &aliases, &mut visited, 0) {
                return Ok(Some(IndexGoto { targets: vec![loc], note: IndexNote::Certain }));
            }
        }

        // A module-level declaration of this file (script-setup state etc.):
        // the scope analysis calls it not-local and the import chain is done.
        if let Some(d) = facts.decls.iter().find(|d| d.name == word) {
            return Ok(Some(IndexGoto {
                targets: vec![Location { path: from.clone().into(), range: d.range, uri: None }],
                note: IndexNote::Certain,
            }));
        }

        // The component registry: a tag without an import (unplugin-style
        // auto-imports) resolves by file name.
        let component = pascal_case(&word);
        if component != word || word.contains('-') {
            let targets: Vec<Location> = index
                .symbols_named(&component, Some(SymbolKind::Class), MAX_WEB_TARGETS)
                .into_iter()
                .filter(|m| m.name == component && is_component_file(m.path.as_ref()))
                .map(|m| Location { path: m.path, range: m.range, uri: None })
                .collect();
            if !targets.is_empty() {
                return Ok(Some(IndexGoto { targets, note: IndexNote::Candidate }));
            }
        }
        Ok(None)
    })
    .await
}

/// The export named `name` of `target`, following re-export chains (depth-
/// and cycle-limited); the file itself for a default export without a
/// declaration, and for a component module.
fn resolve_export(
    index: &ProjectIndex,
    target: &Path,
    name: &str,
    aliases: &AliasMap,
    visited: &mut HashSet<PathBuf>,
    depth: u32,
) -> Option<Location> {
    const MAX_DEPTH: u32 = 8;
    if depth > MAX_DEPTH || !visited.insert(target.to_path_buf()) {
        return None;
    }
    let facts = index.facts_of(target)?;
    // A component module: the file is the component.
    if is_component_file(target) {
        return Some(file_location(index, target));
    }
    // The declaration itself, exported (or default) ones first.
    if let Some(decl) = facts
        .decls
        .iter()
        .find(|d| d.default && name.is_empty())
        .or_else(|| facts.decls.iter().find(|d| d.exported && d.name == name))
        .or_else(|| facts.decls.iter().find(|d| d.name == name))
    {
        return Some(Location { path: target.into(), range: decl.range, uri: None });
    }
    // A re-export names the next module; `export *` re-exports everything.
    for re in &facts.reexports {
        let reexports_name = re.names.iter().any(|n| match n {
            ImportName::Named { imported, .. } => imported == name,
            ImportName::Namespace { local } => local == "*" || local == name,
            ImportName::Default { .. } => false,
        });
        if reexports_name {
            let exists = |p: &Path| index.has_file(p);
            if let Some(next) = resolve_spec(target, &re.source, aliases, &exists)
                && let Some(loc) = resolve_export(index, &next, name, aliases, visited, depth + 1)
            {
                return Some(loc);
            }
        }
    }
    // A default export without a declaration (`export default {…}`): the file.
    if name.is_empty() {
        return Some(file_location(index, target));
    }
    None
}

/// A file-level target: the first declaration of its script, or the start.
fn file_location(index: &ProjectIndex, path: &Path) -> Location {
    let range = index.facts_of(path).and_then(|f| f.decls.first().map(|d| d.range)).unwrap_or_default();
    Location { path: path.into(), range, uri: None }
}

#[command(trust = always)]
pub async fn index_usages(host: &Host, window: &str, name: String, css: bool) -> IdeResult<Vec<Location>> {
    let project = host.project(window)?;
    let index = project.index.clone();
    blocking(move || Ok(index.usages_of(&name, css))).await
}

/// The languages whose `<script>` regions carry module facts.
fn is_web_language(lang: &str) -> bool {
    matches!(lang, "vue" | "svelte" | "html" | "htm" | "jsp")
}

/// Whether `path` is a single-file component module.
fn is_component_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("vue" | "svelte"))
}

/// `my-widget` / `MyWidget` → `MyWidget`: how component tags name files.
fn pascal_case(word: &str) -> String {
    word.split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
        })
        .collect()
}
ide_command::domain!(
    nav:
    find_dirs,
    find_files,
    find_files_named,
    find_symbols,
    find_symbols_named,
    fs_resolve_reference,
    index_file_extensions,
    index_goto,
    index_status,
    index_usages,
    mark_recent,
    nav_index_definitions,
    nav_inheritance,
    nav_resolve_module,
    path_completions,
    recent_files,
    remove_recent,
    web_goto,
    web_unresolved_references,
);
