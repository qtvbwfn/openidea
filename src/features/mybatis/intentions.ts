/**
 * ⌥↵ intentions for Java mapper interfaces: three items listed for every
 * mapper interface — creating the mapper.xml, generating one XML statement,
 * and @Param generation. Each re-checks on click what the mapper already has
 * (jump to the XML or its statement) and what it lacks (create); the menu no
 * longer depends on the index snapshot being resolved. Jumping is also the
 * gutter arrows' / ⌘B's job, so there is no separate jump item. The runs
 * re-check the document version before dispatching edits; creating a file
 * asks for confirmation first (`askCreateXml`).
 */
import { EditorSelection } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { editorsFor, type ContributedIntention, type EditorDocContext } from "@/editor/api";
import { docOf } from "@/editor/lsp/lsp";
import { showHint } from "@/editor/lsp/hint";
import { locationItems, navigateTo } from "@/editor/lsp/navigation";
import { caretAnchor, openPopup } from "@/editor/lsp/popupStore";
import { notifyFileOperation } from "@/platform/fileOps";
import { tx } from "@/platform/i18n";
import { call, commands, type Location } from "@/platform/ipc";
import { dirname, useProject } from "@/platform/project";
import { currentRoots } from "@/platform/projectRoots";
import { editorService } from "@/workbench/contributions";
import { askCreateXml } from "./dialogs";
import { appendStatementEdit, importEdit, mapperXmlContent, paramAnnotationEdits, statementXml } from "./generate";
import { inType, methodAt, parseMapperInterface, type MapperInterface, type MapperMethod } from "./javaParse";
import { parseMapperXml } from "./xmlParse";
import { configuredXmlDirs, ensureMapperIndex, patchMapperIndex, type XmlEntry } from "./xmlIndex";
import { refreshMybatisGutters } from "./gutter";

// ---------------------------------------------------------------------------
// Targets

/** The resources root (absolute), else the project root; null without a project. */
function resourcesBase(): string | null {
  const root = useProject.getState().project?.root ?? null;
  if (!root) return null;
  const res = currentRoots().find((r) => r.kind === "resources");
  return res ? (res.path ? `${root}/${res.path}` : root) : null;
}

function entryFrom(path: string, text: string): XmlEntry {
  const xml = parseMapperXml(text);
  const statements = new Map<
    string,
    { tag: "select" | "insert" | "update" | "delete"; id: string; line: number; col: number }[]
  >();
  for (const s of xml.statements) {
    const list = statements.get(s.id);
    if (list) list.push(s);
    else statements.set(s.id, [s]);
  }
  return { path, statements };
}

/**
 * Where the interface's XML lives / would be created: the index first, then
 * the configured `mapper-locations` directories, the mirrored resources
 * path, and finally next to the Java file. Existing files are checked by
 * namespace (never overwritten; the backend refuses to truncate anyway).
 */
async function xmlTargets(
  iface: MapperInterface,
  javaPath: string,
): Promise<{ existing: XmlEntry[]; createPath: string | null }> {
  const root = useProject.getState().project?.root ?? null;
  const index = await ensureMapperIndex(root);
  const existing = index.xmls.get(iface.fqn) ?? [];
  if (existing.length) return { existing, createPath: null };
  const base = resourcesBase();
  const dirs = await configuredXmlDirs();
  const rel = iface.pkg ? `${iface.pkg.replace(/\./g, "/")}/` : "";
  const candidates: string[] = [];
  for (const d of dirs) {
    const dirBase = base ?? root;
    candidates.push(d ? `${dirBase}/${d}/${iface.name}.xml` : `${dirBase}/${iface.name}.xml`);
  }
  if (base) candidates.push(`${base}/${rel}${iface.name}.xml`);
  candidates.push(`${dirname(javaPath)}/${iface.name}.xml`);
  for (const c of [...new Set(candidates)]) {
    const text = await call(commands.fsReadText(c)).catch(() => null);
    if (text === null) return { existing: [], createPath: c };
    if (parseMapperXml(text).namespace === iface.fqn) return { existing: [entryFrom(c, text)], createPath: null };
  }
  return { existing: [], createPath: null };
}

// ---------------------------------------------------------------------------
// Runs

/** Applies the @Param plan (and the import) to the live document; no-op when it changed. */
function runParams(view: EditorView, iface: MapperInterface, scope: MapperMethod | null) {
  const text = view.state.doc.toString();
  if (parseMapperInterface(text)?.fqn !== iface.fqn) {
    showHint(view, tx("The document changed; retry"), { tone: "error" });
    return;
  }
  const edits = paramAnnotationEdits(iface, scope);
  if (!edits.length) return;
  const ie = importEdit(text);
  if (ie) edits.push(ie);
  view.dispatch({ changes: edits, userEvent: "input" });
}

/** Needs the @Param intention: an empty `@Param()` to fill, or a multi-parameter method with missing annotations. */
const needsParams = (m: MapperMethod) =>
  m.params.some((p) => p.emptyParamAt) || (m.params.length >= 2 && m.params.some((p) => !p.hasParam));

/** @Param for freshly generated multi-parameter statements, so their `#{…}` refs bind. */
function paramsForMissing(
  view: EditorView,
  iface: MapperInterface,
  scope: MapperMethod | null,
  methods: MapperMethod[],
) {
  if (methods.some(needsParams)) runParams(view, iface, scope);
}

/** Opens the XML (or opens it and appends), caret on the first TODO. */
async function appendStatement(path: string, stmt: string): Promise<boolean> {
  await editorService().openFile(path, { focus: true });
  const ed = editorsFor(path)[0];
  if (!ed) return false;
  const view = ed.view;
  const edit = appendStatementEdit(view.state.doc.toString(), stmt);
  const caret = edit.from + Math.max(0, edit.insert.indexOf("<!-- TODO"));
  view.dispatch({
    changes: [edit],
    selection: EditorSelection.cursor(caret),
    scrollIntoView: true,
    userEvent: "input",
  });
  // The edit may be unsaved: merge it into the cached index and refresh the
  // gutters, so the Java mapper picks the new statement up at once.
  patchMapperIndex(path, view.state.doc.toString());
  refreshMybatisGutters();
  return true;
}

async function createMapperXml(path: string, iface: MapperInterface): Promise<void> {
  const content = mapperXmlContent(iface);
  await call(commands.fsCreateFileBytes(path, Array.from(new TextEncoder().encode(content))));
  notifyFileOperation({ kind: "created", paths: [path] });
  // Merge the new file into the resolved index instead of dropping the cache:
  // a reset leaves mapperIndexSnapshot null until the next rebuild, which
  // blanks every mapper intention (jump / generate / create) right after the
  // very flow that was expected to enable them.
  patchMapperIndex(path, content);
  refreshMybatisGutters();
  await editorService().openFile(path, { focus: true });
}

/** Create the mapper.xml (all eligible methods), or append the statement(s) it lacks. */
async function generateStatementOrCreate(
  view: EditorView,
  ctx: EditorDocContext,
  iface: MapperInterface,
  scope: MapperMethod | null,
) {
  const { existing } = await xmlTargets(iface, ctx.path);
  if (existing.length) {
    const have = new Set(existing.flatMap((e) => [...e.statements.keys()]));
    if (scope && have.has(scope.name)) {
      showHint(view, tx("The XML statement already exists"));
      return;
    }
    const missing = scope ? [scope] : iface.methods.filter((m) => m.kind && !m.defaultMethod && !have.has(m.name));
    if (!missing.length) {
      showHint(view, tx("All methods already have statements"));
      return;
    }
    // Several files may share the namespace; appending to the first is fine —
    // MyBatis loads statements from all of them.
    const entry = existing[0]!;
    const stmts = missing.map((m) => statementXml(m, iface)).join("\n\n");
    if (await appendStatement(entry.path, stmts)) paramsForMissing(view, iface, scope, missing);
    return;
  }
  await createXmlFlow(view, ctx, iface, scope);
}

/** The create half of the statement flow: confirm, write, then fill the fresh @Params. */
async function createXmlFlow(
  view: EditorView,
  ctx: EditorDocContext,
  iface: MapperInterface,
  scope: MapperMethod | null,
) {
  const { createPath } = await xmlTargets(iface, ctx.path);
  if (!createPath) {
    showHint(view, tx("No target location for the mapper XML"), { tone: "error" });
    return;
  }
  if (!(await askCreateXml(createPath))) return;
  await createMapperXml(createPath, iface);
  paramsForMissing(
    view,
    iface,
    scope,
    iface.methods.filter((m) => m.kind && !m.defaultMethod),
  );
}

/** Where the method's XML statements live (the resolved index; empty when none). */
async function statementTargets(iface: MapperInterface, m: MapperMethod): Promise<Location[]> {
  const root = useProject.getState().project?.root ?? null;
  const index = await ensureMapperIndex(root);
  const targets: Location[] = [];
  for (const e of index.xmls.get(iface.fqn) ?? [])
    for (const s of e.statements.get(m.name) ?? [])
      targets.push({
        path: e.path,
        range: { start: { line: s.line, col: s.col }, end: { line: s.line, col: s.col + 1 } },
      });
  return targets;
}

/** The method's statements exist: jump to the one, or choose among them. */
async function jumpToStatements(view: EditorView, iface: MapperInterface, m: MapperMethod): Promise<boolean> {
  const targets = await statementTargets(iface, m);
  if (!targets.length) return false;
  if (targets.length === 1) await navigateTo(targets[0]!);
  else
    openPopup({
      kind: "list",
      anchor: caretAnchor(view, view.state.selection.main.head),
      title: tx("Choose MyBatis Statement"),
      items: await locationItems(targets),
    });
  return true;
}

/** `Create Mapper XML…`: open the mapper's XML when it exists, create it when it doesn't. */
async function openOrCreateXml(view: EditorView, ctx: EditorDocContext, iface: MapperInterface) {
  const { existing } = await xmlTargets(iface, ctx.path);
  if (existing.length) {
    await editorService().openFile(existing[0]!.path, { focus: true });
    return;
  }
  await createXmlFlow(view, ctx, iface, null);
}

/** `Generate XML Statement` with the caret on a method: jump when it exists, generate when it doesn't. */
async function generateOrJumpToStatement(
  view: EditorView,
  ctx: EditorDocContext,
  iface: MapperInterface,
  m: MapperMethod,
) {
  if (await jumpToStatements(view, iface, m)) return;
  await generateStatementOrCreate(view, ctx, iface, m);
}

/** `Generate XML Statement` off a method: fill what's missing, or open the XML when all are there. */
async function generateOrOpenXml(view: EditorView, ctx: EditorDocContext, iface: MapperInterface) {
  const root = useProject.getState().project?.root ?? null;
  const index = await ensureMapperIndex(root);
  const entry = (index.xmls.get(iface.fqn) ?? [])[0];
  const have = entry ? new Set(entry.statements.keys()) : new Set<string>();
  const missing = iface.methods.filter((m) => m.kind && !m.defaultMethod && !have.has(m.name));
  if (!entry || missing.length) {
    await generateStatementOrCreate(view, ctx, iface, null);
    return;
  }
  await editorService().openFile(entry.path, { focus: true });
}

/** `Generate @Param Annotations`: fill what's missing; say so when everything is annotated. */
function runParamsOrHint(view: EditorView, iface: MapperInterface, m: MapperMethod | null) {
  if (m && needsParams(m)) {
    runParams(view, iface, m);
    return;
  }
  if (iface.methods.some(needsParams)) {
    runParams(view, iface, null);
    return;
  }
  showHint(view, tx("All parameters already have @Param"));
}

// ---------------------------------------------------------------------------
// Provider

export function mybatisIntentions(view: EditorView): ContributedIntention[] {
  const ctx = docOf(view);
  if (!ctx || ctx.language !== "java") return [];
  const iface = parseMapperInterface(view.state.doc.toString());
  if (!iface) return [];
  const caret = view.state.selection.main.head;
  const m = methodAt(iface, caret);
  if (!inType(iface, caret)) return [];

  // The three stay listed for every mapper interface; each one re-checks on
  // click what the mapper already has (jump) and what it lacks (create).
  const out: ContributedIntention[] = [
    {
      id: "mybatis.createXml",
      label: tx("Create Mapper XML…"),
      run: () => void openOrCreateXml(view, ctx, iface),
    },
    {
      id: "mybatis.statement",
      label: tx("Generate XML Statement"),
      run: () =>
        void (m && m.kind && !m.defaultMethod
          ? generateOrJumpToStatement(view, ctx, iface, m)
          : generateOrOpenXml(view, ctx, iface)),
    },
    {
      id: "mybatis.params",
      label: tx("Generate @Param Annotations"),
      run: () => runParamsOrHint(view, iface, m),
    },
  ];
  return out;
}
