//! Book AST. Every importer produces it, every renderer consumes it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub title: String,
    pub author: String,
    /// BCP 47 language tag, drives hyphenation and quotes. Empty means English.
    #[serde(default)]
    pub language: String,
    /// Text of the copyright page. Empty leaves the page blank.
    #[serde(default)]
    pub copyright: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Vec<Inline>),
    /// One paragraph of a block quote.
    Quote(Vec<Inline>),
    SceneBreak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Footnote(Vec<Inline>),
}
