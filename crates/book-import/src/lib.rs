//! Importers: source formats to the book AST.

use std::collections::HashMap;

use book_model::{Block, Chapter, Inline};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Marks a footnote reference until its definition is known.
/// Definitions often come after the references, so they are resolved at the end.
const NOTE_REF: &str = "\0note:";

/// Parses Markdown. Each `#` heading starts a chapter, `---` is a scene break.
/// Text before the first `#` heading goes into a chapter with an empty title.
pub fn markdown(src: &str) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    // One frame per open inline container (paragraph, emphasis, heading...).
    let mut stack: Vec<Vec<Inline>> = Vec::new();
    let mut notes: HashMap<String, Vec<Inline>> = HashMap::new();
    let mut note: Option<(String, Vec<Inline>)> = None;
    let mut quote_depth = 0;
    let mut item_depth = 0;

    let current = |chapters: &mut Vec<Chapter>| -> usize {
        if chapters.is_empty() {
            chapters.push(Chapter {
                title: String::new(),
                blocks: Vec::new(),
            });
        }
        chapters.len() - 1
    };

    for event in Parser::new_ext(src, Options::ENABLE_FOOTNOTES) {
        // A block starts inside a list item: emit the item's text first, so it keeps
        // its place before the block. The item continues in a new frame.
        if item_depth > 0
            && matches!(
                event,
                Event::Start(Tag::List(_) | Tag::CodeBlock(_) | Tag::BlockQuote(_))
            )
        {
            let inlines = pop(&mut stack);
            emit(&mut chapters, &mut note, quote_depth > 0, inlines);
            stack.push(Vec::new());
        }
        match &event {
            Event::Start(Tag::Item) => item_depth += 1,
            Event::End(TagEnd::Item) => item_depth -= 1,
            _ => {}
        }
        match event {
            // List items and code blocks have no AST block: their text becomes a paragraph.
            Event::Start(
                Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::Emphasis
                | Tag::Strong
                | Tag::Item
                | Tag::CodeBlock(_),
            ) => stack.push(Vec::new()),
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth -= 1,
            Event::Start(Tag::FootnoteDefinition(label)) => {
                note = Some((label.to_string(), Vec::new()))
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                if let Some((label, content)) = note.take() {
                    notes.insert(label, content);
                }
            }
            Event::End(TagEnd::Emphasis) => {
                let inner = pop(&mut stack);
                push(&mut stack, Inline::Emph(inner));
            }
            Event::End(TagEnd::Strong) => {
                let inner = pop(&mut stack);
                push(&mut stack, Inline::Strong(inner));
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                let title = plain_text(&pop(&mut stack));
                chapters.push(Chapter {
                    title,
                    blocks: Vec::new(),
                });
            }
            // shortcut: sub-headings become plain paragraphs, add a Block::Heading when styles need it.
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => {
                let inlines = pop(&mut stack);
                emit(&mut chapters, &mut note, quote_depth > 0, inlines);
            }
            Event::Rule => {
                let i = current(&mut chapters);
                chapters[i].blocks.push(Block::SceneBreak);
            }
            Event::Text(t) | Event::Code(t) => push(&mut stack, Inline::Text(t.into_string())),
            Event::SoftBreak | Event::HardBreak => push(&mut stack, Inline::Text(" ".into())),
            Event::FootnoteReference(label) => push(
                &mut stack,
                Inline::Footnote(vec![Inline::Text(format!("{NOTE_REF}{label}"))]),
            ),
            _ => {}
        }
    }

    for block in chapters.iter_mut().flat_map(|c| c.blocks.iter_mut()) {
        if let Block::Paragraph(inlines) | Block::Quote(inlines) = block {
            resolve_notes(inlines, &notes, false);
        }
    }
    chapters
}

/// Adds a paragraph to the current chapter, or to the footnote being defined.
fn emit(
    chapters: &mut Vec<Chapter>,
    note: &mut Option<(String, Vec<Inline>)>,
    quote: bool,
    inlines: Vec<Inline>,
) {
    // A loose list item holds paragraphs, already emitted: its own frame is empty.
    if inlines.is_empty() {
        return;
    }
    if let Some((_, content)) = note.as_mut() {
        if !content.is_empty() {
            content.push(Inline::Text(" ".into()));
        }
        content.extend(inlines);
        return;
    }
    if chapters.is_empty() {
        chapters.push(Chapter {
            title: String::new(),
            blocks: Vec::new(),
        });
    }
    let block = if quote {
        Block::Quote(inlines)
    } else {
        Block::Paragraph(inlines)
    };
    chapters
        .last_mut()
        .expect("pushed above")
        .blocks
        .push(block);
}

fn push(stack: &mut [Vec<Inline>], inline: Inline) {
    // Text outside any container (a stray event) is dropped on purpose.
    if let Some(top) = stack.last_mut() {
        top.push(inline);
    }
}

fn pop(stack: &mut Vec<Vec<Inline>>) -> Vec<Inline> {
    stack.pop().unwrap_or_default()
}

/// Replaces footnote markers by their definitions. A note referenced from inside
/// another note becomes text in parentheses: books don't nest notes, and it can't loop.
fn resolve_notes(inlines: &mut [Inline], notes: &HashMap<String, Vec<Inline>>, in_note: bool) {
    for inline in inlines {
        match inline {
            Inline::Footnote(content) => {
                let label = match content.as_slice() {
                    [Inline::Text(t)] => t.strip_prefix(NOTE_REF).map(str::to_string),
                    _ => None,
                };
                let Some(label) = label else { continue };
                // A reference without definition keeps its label, so the author sees it.
                let mut definition = notes
                    .get(&label)
                    .cloned()
                    .unwrap_or_else(|| vec![Inline::Text(format!("[^{label}]"))]);
                if in_note {
                    *inline = Inline::Text(format!(" ({})", plain_text(&definition)));
                } else {
                    resolve_notes(&mut definition, notes, true);
                    *content = definition;
                }
            }
            Inline::Emph(c) | Inline::Strong(c) => resolve_notes(c, notes, in_note),
            Inline::Text(_) => {}
        }
    }
}

fn plain_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text(t) => t.clone(),
            Inline::Emph(c) | Inline::Strong(c) => plain_text(c),
            Inline::Footnote(_) => String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_code_blocks_keep_their_text() {
        let ch =
            markdown("# Un\n\n- alpha\n- beta\n\n```\ncode line\n```\n\n1. loose\n\n2. list\n");
        let text: Vec<String> = ch[0]
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(i) => plain_text(i),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(text, ["alpha", "beta", "code line\n", "loose", "list"]);
    }

    #[test]
    fn nested_lists_keep_their_order() {
        let ch = markdown("- outer\n  - inner\n- next\n");
        let text: Vec<String> = ch[0]
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(i) => plain_text(i),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(text, ["outer", "inner", "next"]);
    }

    #[test]
    fn blocks_inside_an_item_come_after_its_text() {
        let ch = markdown("- outer\n  ```\n  code\n  ```\n- quoted\n  > cited\n");
        let blocks: Vec<String> = ch[0]
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(i) => format!("p:{}", plain_text(i)),
                Block::Quote(i) => format!("q:{}", plain_text(i)),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(blocks, ["p:outer", "p:code\n", "p:quoted", "q:cited"]);
    }

    #[test]
    fn a_note_inside_a_note_leaves_no_marker() {
        let ch = markdown("Texte[^a].\n\n[^a]: Note avec renvoi[^b].\n[^b]: autre[^a].\n");
        let Block::Paragraph(inl) = &ch[0].blocks[0] else {
            panic!()
        };
        assert_eq!(
            inl[1],
            Inline::Footnote(vec![
                Inline::Text("Note avec renvoi".into()),
                Inline::Text(" (autre.)".into()),
                Inline::Text(".".into())
            ])
        );
        assert!(
            !format!("{ch:?}").contains('\0'),
            "no marker may reach the AST"
        );
    }

    #[test]
    fn chapters_scene_breaks_and_notes() {
        let src = "Avant-propos.\n\n# Un\n\nTexte *vif*[^a].\n\n---\n\n> Cité.\n\n# Deux\n\nFin.\n\n[^a]: La note.\n";
        let ch = markdown(src);
        assert_eq!(
            ch.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            ["", "Un", "Deux"]
        );
        assert_eq!(
            ch[1].blocks,
            [
                Block::Paragraph(vec![
                    Inline::Text("Texte ".into()),
                    Inline::Emph(vec![Inline::Text("vif".into())]),
                    Inline::Footnote(vec![Inline::Text("La note.".into())]),
                    Inline::Text(".".into()),
                ]),
                Block::SceneBreak,
                Block::Quote(vec![Inline::Text("Cité.".into())]),
            ]
        );
        assert_eq!(
            ch[2].blocks.len(),
            1,
            "the note definition must not leak as a paragraph"
        );
    }
}
