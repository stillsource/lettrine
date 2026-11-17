import type { JSONContent } from "@tiptap/react";

/** Mirrors `book_project::Meta`. */
export type Meta = {
  title: string;
  author: string;
  language: string;
  copyright: string;
  style: string;
  trim: [number, number];
  bleed: boolean;
  chapters: string[];
  draft: string;
  /** Print style overrides. null keeps the style's value. */
  font: string | null;
  size: number | null;
  leading: number | null;
  cover: CoverSettings;
};

/** Mirrors `book_project::CoverSettings`. */
export type CoverSettings = {
  paper: "white" | "cream" | "color";
  /** Spine width in inches from the printer, replaces the computed one. */
  spine: number | null;
  blurb: string;
  background: string;
  ink: string;
  image: string | null;
};

export type Issue = { level: "error" | "warning" | "info"; message: string };

export type ChapterFile = { title: string; doc: JSONContent };
export type Project = { dir: string; meta: Meta; chapters: ChapterFile[] };
export type Options = { styles: string[]; trims: [string, number, number][]; fonts: string[] };
export type Snapshot = { id: number; draft: string; words: number };
