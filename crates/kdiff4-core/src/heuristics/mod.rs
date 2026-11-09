//! Heuristics Alignment: finds the functions, methods, types and blocks of the source documents and pairs them across the documents, so they can be aligned as manual alignments.

mod extract;
mod matching;

use std::ops::Range;

use kdiff4_protocol::{EntityKind, Extractor, HeuristicOptions, MatchStage};

use crate::error::CoreError;


/// One alignment Heuristics Alignment found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    pub kind: EntityKind,
    /// Per document, the qualified name of its entity, or `None` where it has an anchor.
    pub names: Vec<Option<String>>,
    pub parts: Vec<Range<usize>>,
    pub stage: MatchStage,
    /// The index in the list of the alignment whose entities hold these.
    pub parent: Option<usize>,
}


/// The text of one source document and the byte ranges of its lines.
pub struct DocumentLines<'a> {
    pub text: &'a str,
    pub lines: &'a [Range<usize>],
}


/// The alignments Heuristics Alignment finds in `documents` of `language`, depth-first by position in the first document, and how many entities each document has at any depth. With `within`, one range of lines per document, only the entities inside those ranges are paired; when each document has a single one there, its children are paired instead, and so on down. Blocks are always looked for then.
pub fn propose(language: &str, documents: &[DocumentLines<'_>], options: &HeuristicOptions, within: Option<&[Range<usize>]>) -> Result<(Vec<Proposed>, Vec<usize>), CoreError> {
    if !extract::supports(options.extractor.clone(), language) {
        return Err(CoreError::UnsupportedLanguage { language: language.to_string() });
    }
    let kinds: Vec<EntityKind> = [
        (options.functions, EntityKind::Function),
        (options.methods, EntityKind::Method),
        (options.types, EntityKind::Type),
        (options.blocks || within.is_some(), EntityKind::Block),
    ]
    .into_iter()
    .filter(|(wanted, _)| *wanted)
    .map(|(_, kind)| kind)
    .collect();
    let mut entities: Vec<Vec<extract::Entity>> = documents
        .iter()
        .map(|document| extract::extract(options.extractor.clone(), language, document.text, document.lines, &kinds).unwrap_or_default())
        .collect();
    if let Some(parts) = within {
        entities = scoped(entities, parts);
    }
    let found = entities.iter().map(|entities| count(entities)).collect();
    let proposed = matching::align(&entities, options)
        .into_iter()
        .map(|aligned| Proposed {
            kind: aligned.kind,
            names: aligned.names,
            parts: aligned.parts,
            stage: aligned.stage,
            parent: aligned.parent,
        })
        .collect();
    Ok((proposed, found))
}


/// The functions, methods, types and blocks of one document of `language` at every depth, each as the lines from its header to its closing line, sorted by start. `None` when the tree-sitter extractor can't parse `language`.
pub fn entity_spans(language: &str, document: &DocumentLines<'_>) -> Option<Vec<Range<usize>>> {
    let kinds = [EntityKind::Function, EntityKind::Method, EntityKind::Type, EntityKind::Block];
    let entities = extract::extract(Extractor::TreeSitter, language, document.text, document.lines, &kinds)?;
    let mut spans = Vec::new();
    collect_spans(&entities, &mut spans);
    spans.sort_by_key(|span: &Range<usize>| (span.start.clone(), std::cmp::Reverse(span.end.clone())));
    Some(spans)
}


fn collect_spans(entities: &[extract::Entity], spans: &mut Vec<Range<usize>>) {
    for entity in entities {
        spans.push(entity.header.clone()..entity.lines.end.clone());
        collect_spans(&entity.children, spans);
    }
}


fn count(entities: &[extract::Entity]) -> usize {
    entities.iter().map(|entity| 1 + count(&entity.children)).sum()
}


/// The outermost entities inside `parts` per document, replaced by their children while every document has exactly one. A single block without children stays, since nothing is inside it to pair.
fn scoped(entities: Vec<Vec<extract::Entity>>, parts: &[Range<usize>]) -> Vec<Vec<extract::Entity>> {
    let mut candidates: Vec<Vec<extract::Entity>> = entities.iter().zip(parts.iter()).map(|(entities, part)| inside(entities, part)).collect();
    while candidates.iter().all(|candidates| candidates.len() == 1) && !candidates.iter().all(|candidates| candidates[0].kind == EntityKind::Block && candidates[0].children.is_empty()) {
        candidates = candidates.into_iter().map(|mut candidates| candidates.remove(0).children).collect();
    }
    candidates
}


fn inside(entities: &[extract::Entity], part: &Range<usize>) -> Vec<extract::Entity> {
    entities
        .iter()
        .flat_map(|entity| {
            if part.start <= entity.lines.start && entity.lines.end <= part.end {
                vec![entity.clone()]
            } else if entity.lines.start < part.end && part.start < entity.lines.end {
                inside(&entity.children, part)
            } else {
                Vec::new()
            }
        })
        .collect()
}
