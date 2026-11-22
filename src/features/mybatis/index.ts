/**
 * MyBatis support: generation of the mapper.xml / single statements / @Param
 * annotations through ⌥↵ intentions, and the SQL-log conversion in the Run
 * console. Everything is text-based: no language server, no Rust changes.
 *
 * Navigation (the gutter arrows and Go to Definition) lives in the MyBatis Duo
 * plugin, whose document indexer owns the Java ↔ XML graph; this feature used
 * to draw its own gutter and drew a second arrow next to the plugin's.
 */
import { intentionProviders, registerEditorContribution } from "@/editor/api";
import { onIdeEvent } from "@/platform/ipc";
import { registerOverlay } from "@/workbench/contributions";
import { CreateXmlDialogHost } from "./dialogs";
import { mybatisIntentions } from "./intentions";
import { resetMapperIndex } from "./xmlIndex";

const MAPPER_PATH = /\.(java|xml)$/;

export function activate(): () => void {
  // Saves and external edits rebuild the project index, so the generation
  // intentions see current mapper files (`fsChanged` is already debounced by
  // the backend).
  let fsTimer: ReturnType<typeof setTimeout> | null = null;
  const offFs = onIdeEvent("fsChanged", (e) => {
    const touches = [...e.created, ...e.modified, ...e.removed].some((p) => MAPPER_PATH.test(p));
    if (!touches) return;
    if (fsTimer) clearTimeout(fsTimer);
    fsTimer = setTimeout(() => resetMapperIndex(), 400);
  });
  const disposers = [
    registerEditorContribution({
      id: "mybatis",
      order: 58, // after the LSP line markers (56), before templates (90)
      extensions: () => intentionProviders.of(mybatisIntentions),
    }),
    registerOverlay({ id: "mybatis.createXml", component: CreateXmlDialogHost }),
  ];
  return () => {
    offFs();
    if (fsTimer) clearTimeout(fsTimer);
    disposers.reverse().forEach((d) => d());
  };
}
