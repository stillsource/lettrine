import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Footnote } from "./footnote";
import { Preview } from "./Preview";
import type { ChapterFile, CoverSettings, Issue, Meta, Options, Project, Snapshot } from "./types";

const SAVE_DELAY = 500;
const PREVIEW_DELAY = 1000;

export function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [options, setOptions] = useState<Options>({ styles: [], trims: [], fonts: [] });
  // Bumped when the project is replaced on disk (restore, draft switch), to remount the editor.
  const [session, setSession] = useState(0);
  const [error, setError] = useState("");

  // Stable functions: effects depend on them, and must not rerun on each render.
  const report = useCallback((e: unknown) => setError(String(e)), []);
  const load = useCallback(
    (p: Promise<Project>) =>
      p.then((next) => {
        setProject(next);
        setSession((n) => n + 1);
      }, report),
    [report],
  );

  useEffect(() => {
    invoke<Options>("options").then(setOptions, report);
    invoke<string | null>("startup_project").then((dir) => {
      if (dir) load(invoke("open_project", { dir }));
    }, report);
  }, []);

  return (
    <>
      {error && (
        <div className="error" onClick={() => setError("")}>
          {error}
        </div>
      )}
      {project ? (
        <Workspace key={`${project.dir}#${session}`} initial={project} options={options} onError={report} onReplace={load} />
      ) : (
        <Start onProject={load} />
      )}
    </>
  );
}

function Start({ onProject }: { onProject: (p: Promise<Project>) => void }) {
  const openProject = async () => {
    const dir = await open({ directory: true, title: "Ouvrir un projet" });
    if (dir) onProject(invoke("open_project", { dir }));
  };
  const newProject = async (fromMarkdown: boolean) => {
    const markdown = fromMarkdown
      ? await open({ title: "Manuscrit Markdown", filters: [{ name: "Markdown", extensions: ["md", "markdown", "txt"] }] })
      : null;
    if (fromMarkdown && !markdown) return;
    const dir = await save({ title: "Dossier du nouveau projet" });
    if (!dir) return;
    const title = dir.split(/[\\/]/).pop() ?? "Sans titre";
    onProject(invoke("new_project", { dir, title, markdown }));
  };
  return (
    <div className="start">
      <h1>Lettrine</h1>
      <button onClick={openProject}>Ouvrir un projet…</button>
      <button onClick={() => newProject(false)}>Nouveau projet…</button>
      <button onClick={() => newProject(true)}>Importer un manuscrit Markdown…</button>
    </div>
  );
}

type WorkspaceProps = {
  initial: Project;
  options: Options;
  onError: (e: unknown) => void;
  /** The project changed on disk: reload the whole workspace. */
  onReplace: (p: Promise<Project>) => void;
};

function Workspace({ initial, options, onError, onReplace }: WorkspaceProps) {
  const [project, setProject] = useState(initial);
  const [current, setCurrent] = useState(0);
  const [pages, setPages] = useState<string[]>([]);
  const [rendering, setRendering] = useState(false);
  const [note, setNote] = useState("");

  // What changed since the last save. Refs, so the close handler sees the latest values.
  const dirty = useRef({ meta: false, chapters: new Set<number>() });
  const latest = useRef(project);
  latest.current = project;
  const currentRef = useRef(current);
  currentRef.current = current;

  // Saves run one after the other. A flush awaited before a draft switch or a
  // restore also waits for the autosave in progress, so nothing is written after.
  const saving = useRef<Promise<void>>(Promise.resolve());
  const flush = useCallback(() => {
    const run = saving.current
      .catch(() => {})
      .then(async () => {
        const { dir, meta, chapters } = latest.current;
        const pending = dirty.current;
        dirty.current = { meta: false, chapters: new Set() };
        let metaLeft = pending.meta;
        const chaptersLeft = new Set(pending.chapters);
        try {
          if (metaLeft) await invoke("save_meta", { dir, meta });
          metaLeft = false;
          for (const i of pending.chapters) {
            const warning = await invoke<string | null>("save_chapter", { dir, file: meta.chapters[i], chapter: chapters[i] });
            if (warning) onError(`Texte enregistré, mais l'historique n'a pas pu être sauvegardé : ${warning}`);
            chaptersLeft.delete(i);
          }
        } catch (e) {
          // Keep what was not saved, so the next flush retries it.
          dirty.current.meta ||= metaLeft;
          chaptersLeft.forEach((i) => dirty.current.chapters.add(i));
          throw e;
        }
      });
    saving.current = run;
    return run;
  }, [onError]);

  // Set while the backend replaces the project: edits made then would be
  // saved into the new draft, so they are refused.
  const frozen = useRef(false);

  const updateMeta = (patch: Partial<Meta>) => {
    if (frozen.current) return;
    dirty.current.meta = true;
    setProject((p) => ({ ...p, meta: { ...p.meta, ...patch } }));
  };
  const updateChapter = (i: number, patch: Partial<ChapterFile>) => {
    if (frozen.current) return;
    dirty.current.chapters.add(i);
    setProject((p) => ({ ...p, chapters: p.chapters.map((c, j) => (j === i ? { ...c, ...patch } : c)) }));
  };

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: false,
        codeBlock: false,
        code: false,
        bulletList: false,
        orderedList: false,
        listItem: false,
        strike: false,
      }),
      Footnote,
    ],
    content: project.chapters[0]?.doc,
    onUpdate: ({ editor }) => updateChapter(currentRef.current, { doc: editor.getJSON() }),
  });

  useEffect(() => {
    editor?.commands.setContent(latest.current.chapters[current]?.doc ?? "", { emitUpdate: false });
  }, [editor, current]);

  // Autosave.
  useEffect(() => {
    const t = setTimeout(() => flush().catch(onError), SAVE_DELAY);
    return () => clearTimeout(t);
  }, [project, flush, onError]);

  // Live preview from the editor state. Late answers to older requests are dropped.
  const request = useRef(0);
  useEffect(() => {
    const t = setTimeout(() => {
      const id = ++request.current;
      setRendering(true);
      invoke<string[]>("preview", { meta: project.meta, chapters: project.chapters })
        .then((svgs) => id === request.current && setPages(svgs), onError)
        .finally(() => id === request.current && setRendering(false));
    }, PREVIEW_DELAY);
    return () => clearTimeout(t);
  }, [project, onError]);

  // Save pending edits before the window closes.
  useEffect(() => {
    const win = getCurrentWindow();
    const unlisten = win.onCloseRequested(async (event) => {
      event.preventDefault();
      try {
        await flush();
      } catch (e) {
        onError(`Fermeture annulée, des modifications ne sont pas enregistrées : ${e}`);
        return;
      }
      await win.destroy().catch(onError);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [flush, onError]);

  const addChapter = async () => {
    if (frozen.current) return;
    await flush();
    const next = await invoke<Project>("add_chapter", { dir: project.dir });
    setProject(next);
    setCurrent(next.chapters.length - 1);
  };

  /** Saves pending edits, then lets the backend replace the project on disk. */
  const replaceWith = async (command: string, args: Record<string, unknown>) => {
    if (frozen.current) return;
    frozen.current = true;
    editor?.setEditable(false, false);
    try {
      await flush();
    } catch (e) {
      // Nothing changed on disk yet: give the editor back.
      frozen.current = false;
      editor?.setEditable(true, false);
      throw e;
    }
    // From here the backend may change the disk even if it then fails. Reload from
    // disk instead of keeping the old state, which the next autosave would write back.
    const next = invoke<Project>(command, { dir: project.dir, ...args }).catch((e) => {
      onError(e);
      return invoke<Project>("open_project", { dir: project.dir });
    });
    onReplace(next);
  };

  const exportAs = async (kind: "pdf" | "epub" | "cover") => {
    const ext = kind === "epub" ? "epub" : "pdf";
    const title = kind === "cover" ? "Exporter la couverture" : `Exporter en ${ext.toUpperCase()}`;
    const path = await save({ title, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
    if (!path) return;
    setExporting(true);
    await invoke("export", { dir: project.dir, meta: project.meta, chapters: project.chapters, [kind]: path })
      .catch(onError)
      .finally(() => setExporting(false));
  };
  const [exporting, setExporting] = useState(false);
  const [tab, setTab] = useState<"pages" | "cover">("pages");

  const updateCover = (patch: Partial<CoverSettings>) => updateMeta({ cover: { ...project.meta.cover, ...patch } });
  const pickCoverImage = async () => {
    if (frozen.current) return;
    const path = await open({ title: "Image de couverture", filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg"] }] });
    if (!path) return;
    const image = await invoke<string>("import_cover_image", { dir: project.dir, path });
    updateCover({ image });
  };

  const insertNote = () => {
    if (frozen.current || !note.trim()) return;
    editor?.chain().focus().insertContent({ type: "footnote", attrs: { text: note.trim() } }).run();
    setNote("");
  };

  const { meta } = project;
  const trimValue = `${meta.trim[0]}x${meta.trim[1]}`;

  return (
    <div className="workspace">
      <aside className="sidebar">
        <h2>Chapitres</h2>
        <ol>
          {project.chapters.map((c, i) => (
            <li key={meta.chapters[i]} className={i === current ? "active" : ""} onClick={() => setCurrent(i)}>
              {c.title || <em>Sans titre</em>}
            </li>
          ))}
        </ol>
        <button onClick={() => addChapter().catch(onError)}>+ Chapitre</button>

        <h2>Livre</h2>
        <label>
          Titre
          <input value={meta.title} onChange={(e) => updateMeta({ title: e.target.value })} />
        </label>
        <label>
          Auteur
          <input value={meta.author} onChange={(e) => updateMeta({ author: e.target.value })} />
        </label>
        <label>
          Langue
          <input value={meta.language} placeholder="fr" onChange={(e) => updateMeta({ language: e.target.value })} />
        </label>
        <label>
          Copyright
          <textarea value={meta.copyright} onChange={(e) => updateMeta({ copyright: e.target.value })} />
        </label>
        <label>
          Style
          <select value={meta.style} onChange={(e) => updateMeta({ style: e.target.value })}>
            {options.styles.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </label>
        <label>
          Format
          <select
            value={trimValue}
            onChange={(e) => {
              const [w, h] = e.target.value.split("x").map(Number);
              updateMeta({ trim: [w, h] });
            }}
          >
            {!options.trims.some(([, w, h]) => `${w}x${h}` === trimValue) && <option value={trimValue}>{trimValue} in</option>}
            {options.trims.map(([name, w, h]) => (
              <option key={name} value={`${w}x${h}`}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <label className="inline">
          <input type="checkbox" checked={meta.bleed} onChange={(e) => updateMeta({ bleed: e.target.checked })} />
          Fond perdu
        </label>

        <label>
          Police
          <select value={meta.font ?? ""} onChange={(e) => updateMeta({ font: e.target.value || null })}>
            <option value="">Celle du style</option>
            {options.fonts.map((f) => (
              <option key={f}>{f}</option>
            ))}
          </select>
        </label>
        <div className="row">
          <label>
            Corps (pt)
            <NumberInput value={meta.size} step={0.5} min={6} max={24} onChange={(size) => updateMeta({ size })} />
          </label>
          <label>
            Interligne (em)
            <NumberInput value={meta.leading} step={0.05} min={0.2} max={2} onChange={(leading) => updateMeta({ leading })} />
          </label>
        </div>

        <Drafts dir={project.dir} active={meta.draft} onError={onError} replaceWith={replaceWith} />
        <History dir={project.dir} onError={onError} replaceWith={replaceWith} />

        <details>
          <summary>Couverture</summary>
          <label>
            Papier
            <select value={meta.cover.paper} onChange={(e) => updateCover({ paper: e.target.value as CoverSettings["paper"] })}>
              <option value="white">Blanc</option>
              <option value="cream">Crème</option>
              <option value="color">Couleur</option>
            </select>
          </label>
          <label>
            Dos (in), si l'imprimeur le donne
            <NumberInput value={meta.cover.spine} step={0.001} min={0} max={4} onChange={(spine) => updateCover({ spine })} />
          </label>
          <p className="hint">Vide : calculé avec la formule KDP. Pour IngramSpark, reprenez la valeur de leur gabarit.</p>
          <div className="row">
            <label>
              Fond
              <input type="color" value={meta.cover.background} onChange={(e) => updateCover({ background: e.target.value })} />
            </label>
            <label>
              Texte
              <input type="color" value={meta.cover.ink} onChange={(e) => updateCover({ ink: e.target.value })} />
            </label>
          </div>
          <label>
            Quatrième de couverture
            <textarea rows={4} value={meta.cover.blurb} onChange={(e) => updateCover({ blurb: e.target.value })} />
          </label>
          <div className="row">
            <button onClick={() => pickCoverImage().catch(onError)}>{meta.cover.image ? "Changer l'image…" : "Image de 1re…"}</button>
            {meta.cover.image && <button onClick={() => updateCover({ image: null })}>Retirer</button>}
          </div>
        </details>

        <Health project={project} onError={onError} />

        <h2>Export</h2>
        <button disabled={exporting} onClick={() => exportAs("pdf")}>PDF imprimable…</button>
        <button disabled={exporting} onClick={() => exportAs("cover")}>Couverture PDF…</button>
        <button disabled={exporting} onClick={() => exportAs("epub")}>EPUB…</button>
        {exporting && <p className="hint">Export en cours…</p>}
      </aside>

      <main className="editor-pane">
        <div className="toolbar">
          <button onClick={() => editor?.chain().focus().toggleBold().run()}>
            <b>G</b>
          </button>
          <button onClick={() => editor?.chain().focus().toggleItalic().run()}>
            <i>I</i>
          </button>
          <button onClick={() => editor?.chain().focus().toggleBlockquote().run()}>Citation</button>
          <button onClick={() => editor?.chain().focus().setHorizontalRule().run()}>Changement de scène</button>
          <input
            value={note}
            placeholder="Texte de la note"
            onChange={(e) => setNote(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && insertNote()}
          />
          <button onClick={insertNote}>Note</button>
        </div>
        <input
          className="chapter-title"
          placeholder="Titre du chapitre"
          value={project.chapters[current]?.title ?? ""}
          onChange={(e) => updateChapter(current, { title: e.target.value })}
        />
        <EditorContent editor={editor} className="editor" />
      </main>

      <section className="preview-pane">
        <div className="preview-status">
          <button className={tab === "pages" ? "tab active" : "tab"} onClick={() => setTab("pages")}>
            Pages
          </button>
          <button className={tab === "cover" ? "tab active" : "tab"} onClick={() => setTab("cover")}>
            Couverture
          </button>
          <span>{rendering ? "Mise en page…" : `${pages.length} pages`}</span>
        </div>
        {tab === "pages" ? <Preview pages={pages} /> : <CoverPreview project={project} onError={onError} />}
      </section>
    </div>
  );
}

/** An optional number: empty means "use the style's value". The text is kept
 * locally so partial input like "1" on the way to "14" is not rejected. */
function NumberInput(props: { value: number | null; step: number; min: number; max: number; onChange: (v: number | null) => void }) {
  const { value, onChange, ...limits } = props;
  const [text, setText] = useState(value?.toString() ?? "");
  useEffect(() => {
    setText((t) => (Number(t) === value || (t === "" && value === null) ? t : (value?.toString() ?? "")));
  }, [value]);
  return (
    <input
      type="number"
      placeholder="style"
      value={text}
      {...limits}
      onChange={(e) => {
        setText(e.target.value);
        const v = e.target.valueAsNumber;
        if (e.target.value === "") onChange(null);
        else if (v >= limits.min && v <= limits.max) onChange(v);
      }}
    />
  );
}

type Replace = (command: string, args: Record<string, unknown>) => Promise<void>;

function Drafts({ dir, active, onError, replaceWith }: { dir: string; active: string; onError: (e: unknown) => void; replaceWith: Replace }) {
  const [drafts, setDrafts] = useState<string[]>([]);
  const [name, setName] = useState("");
  useEffect(() => {
    invoke<string[]>("drafts", { dir }).then(setDrafts, onError);
  }, [dir, active, onError]);

  return (
    <details>
      <summary>Brouillons</summary>
      <ul className="plain">
        {drafts.map((d) =>
          d === active ? (
            <li key={d} className="active">
              {d}
            </li>
          ) : (
            <li key={d}>
              {d} <button onClick={() => replaceWith("switch_draft", { name: d }).catch(onError)}>Ouvrir</button>
            </li>
          ),
        )}
      </ul>
      <div className="row">
        <input value={name} placeholder="Nom du nouveau brouillon" onChange={(e) => setName(e.target.value)} />
        <button
          disabled={!name.trim()}
          onClick={() => replaceWith("new_draft", { name: name.trim() }).then(() => setName(""), onError)}
        >
          Créer
        </button>
      </div>
      <p className="hint">Le brouillon actuel est gardé tel quel. Vous continuez sur la copie.</p>
    </details>
  );
}

function History({ dir, onError, replaceWith }: { dir: string; onError: (e: unknown) => void; replaceWith: Replace }) {
  const [list, setList] = useState<Snapshot[]>([]);
  const refresh = () => invoke<Snapshot[]>("snapshots", { dir }).then(setList, onError);

  const restore = async (s: Snapshot) => {
    const when = new Date(s.id * 1000).toLocaleString();
    const ok = await ask(`Revenir à l'état du ${when} ? L'état actuel est sauvegardé avant.`, {
      title: "Restaurer",
      kind: "warning",
      okLabel: "Restaurer",
      cancelLabel: "Annuler",
    });
    if (ok) await replaceWith("restore_snapshot", { id: s.id }).catch(onError);
  };

  return (
    <details onToggle={(e) => (e.currentTarget as HTMLDetailsElement).open && refresh()}>
      <summary>Historique</summary>
      {list.length === 0 && <p className="hint">Aucune sauvegarde pour l'instant. Une est prise toutes les 10 minutes d'écriture.</p>}
      <ul className="plain">
        {list.map((s) => (
          <li key={s.id}>
            <span>
              {new Date(s.id * 1000).toLocaleString()}
              <br />
              <small>
                {s.draft} · {s.words} mots
              </small>
            </span>
            <button onClick={() => restore(s)}>Restaurer</button>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** The full cover, recomputed when the book changes. */
function CoverPreview({ project, onError }: { project: Project; onError: (e: unknown) => void }) {
  const [url, setUrl] = useState("");
  const request = useRef(0);
  useEffect(() => {
    const t = setTimeout(() => {
      const id = ++request.current;
      invoke<string>("cover_preview", { dir: project.dir, meta: project.meta, chapters: project.chapters }).then((svg) => {
        if (id === request.current) setUrl(URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" })));
      }, onError);
    }, PREVIEW_DELAY);
    return () => clearTimeout(t);
  }, [project, onError]);
  useEffect(() => () => URL.revokeObjectURL(url), [url]);
  return <div className="preview">{url && <img className="cover" src={url} alt="Couverture" />}</div>;
}

const LEVELS: Record<Issue["level"], string> = { error: "Bloquant", warning: "À revoir", info: "Info" };

/** Book Health. Typesets the whole book, so it runs on demand. */
function Health({ project, onError }: { project: Project; onError: (e: unknown) => void }) {
  const [issues, setIssues] = useState<Issue[] | null>(null);
  const [running, setRunning] = useState(false);
  const run = () => {
    setRunning(true);
    invoke<Issue[]>("check", { dir: project.dir, meta: project.meta, chapters: project.chapters })
      .then(setIssues, onError)
      .finally(() => setRunning(false));
  };
  return (
    <details onToggle={(e) => (e.currentTarget as HTMLDetailsElement).open && issues === null && run()}>
      <summary>Santé du livre</summary>
      <button disabled={running} onClick={run}>
        {running ? "Vérification…" : "Vérifier"}
      </button>
      {issues?.length === 0 && <p className="hint">Rien à signaler.</p>}
      <ul className="plain issues">
        {issues?.map((i, n) => (
          <li key={n} className={i.level}>
            <strong>{LEVELS[i.level]}</strong> {i.message}
          </li>
        ))}
      </ul>
    </details>
  );
}
