import { Node } from "@tiptap/core";

/** A footnote, kept as plain text in an attribute. Shown as a superscript star. */
export const Footnote = Node.create({
  name: "footnote",
  group: "inline",
  inline: true,
  atom: true,
  addAttributes() {
    return { text: { default: "" } };
  },
  parseHTML() {
    return [{ tag: "sup[data-footnote]", getAttrs: (el) => ({ text: (el as HTMLElement).dataset.footnote ?? "" }) }];
  },
  renderHTML({ node }) {
    return ["sup", { "data-footnote": node.attrs.text, title: node.attrs.text, class: "footnote" }, "*"];
  },
});
