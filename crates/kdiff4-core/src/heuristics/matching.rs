use std::collections::HashMap;
use std::ops::Range;

use kdiff4_protocol::{EntityKind, HeuristicOptions, MatchStage};

use super::extract::Entity;

/// Signatures are compared by their first characters only, which keeps the edit distance cheap for long ones.
const SIGNATURE_LIMIT: usize = 256;


/// Entities of the documents that belong together, with the lines each document aligns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aligned {
    pub kind: EntityKind,
    /// Per document, the index of its entity, or `None` where the document lacks it.
    pub entities: Vec<Option<usize>>,
    /// Per document, the lines of its entity, or an anchor where it lacks it.
    pub parts: Vec<Range<usize>>,
    pub stage: MatchStage,
    /// Per document, the name of its entity, or `None` where it lacks it.
    pub names: Vec<Option<String>>,
    /// The index in the list `align` returns of the alignment whose entities hold these.
    pub parent: Option<usize>,
}


/// Pairs the entities of 2 or 3 documents, then the children of every pair that has an entity in each document, and so on. The list is depth-first: a pair comes right before its children, and the pairs of each level are ordered by position in the first document. See `align_level` for how a level is paired. Of the blocks among the children of one pair, only the largest set that is in the same order in every document stays.
pub fn align(documents: &[Vec<Entity>], options: &HeuristicOptions) -> Vec<Aligned> {
    let levels: Vec<&[Entity]> = documents.iter().map(Vec::as_slice).collect();
    let mut result = Vec::new();
    descend(&levels, options, None, &mut result);
    result
}


fn descend(levels: &[&[Entity]], options: &HeuristicOptions, parent: Option<usize>, result: &mut Vec<Aligned>) {
    let aligned = in_order(align_level(levels, options));
    for entry in aligned {
        let index = result.len();
        let children: Option<Vec<&[Entity]>> = entry
            .entities
            .iter()
            .zip(levels.iter())
            .map(|(entity, level)| entity.and_then(|entity| level.get(entity)).map(|entity| entity.children.as_slice()))
            .collect();
        result.push(Aligned { parent, ..entry });
        if let Some(children) = children {
            descend(&children, options, Some(index), result);
        }
    }
}


/// Drops the blocks of `aligned` that are not in the order of the largest set of blocks that every document has in the same order, so blocks never move.
fn in_order(aligned: Vec<Aligned>) -> Vec<Aligned> {
    let blocks: Vec<usize> = (0..aligned.len()).filter(|index| aligned[*index].kind == EntityKind::Block).collect();
    let follows = |before: &Aligned, after: &Aligned| before.parts.iter().zip(after.parts.iter()).all(|(before, after)| before.end <= after.start);
    let mut longest: Vec<usize> = vec![1; blocks.len()];
    let mut previous: Vec<Option<usize>> = vec![None; blocks.len()];
    for to in 0..blocks.len() {
        for from in 0..to {
            if follows(&aligned[blocks[from]], &aligned[blocks[to]]) && longest[from] + 1 > longest[to] {
                longest[to] = longest[from] + 1;
                previous[to] = Some(from);
            }
        }
    }
    let mut kept = vec![false; aligned.len()];
    let mut current = (0..blocks.len()).max_by_key(|index| (longest[*index], std::cmp::Reverse(*index)));
    while let Some(index) = current {
        kept[blocks[index]] = true;
        current = previous[index];
    }
    aligned.into_iter().enumerate().filter(|(index, entry)| entry.kind != EntityKind::Block || kept[*index]).map(|(_, entry)| entry).collect()
}


/// Pairs the entities of 2 or 3 documents, ordered by position in the first document. In 3 documents an entity found in all of them pairs through the first one; an entity one document lacks is aligned at an anchor between its neighbors when `options` allow it and the neighbors are neighbors in that document too.
fn align_level(documents: &[&[Entity]], options: &HeuristicOptions) -> Vec<Aligned> {
    let every = |entities: &[Entity]| vec![true; entities.len()];
    let mut found: Vec<(Vec<Option<usize>>, MatchStage)> = match documents {
        [a, b] => pair(a, &every(a), b, &every(b), options)
            .into_iter()
            .map(|(first, second, stage)| (vec![Some(first), Some(second)], stage))
            .collect(),
        [a, b, c] => {
            let ab: HashMap<usize, (usize, MatchStage)> = pair(a, &every(a), b, &every(b), options)
                .into_iter()
                .map(|(first, second, stage)| (first, (second, stage)))
                .collect();
            let ac: HashMap<usize, (usize, MatchStage)> = pair(a, &every(a), c, &every(c), options)
                .into_iter()
                .map(|(first, second, stage)| (first, (second, stage)))
                .collect();
            let mut found = Vec::new();
            for index in 0..a.len() {
                match (ab.get(&index), ac.get(&index)) {
                    (Some((b_index, b_stage)), Some((c_index, c_stage))) => found.push((vec![Some(index), Some(b_index.clone()), Some(c_index.clone())], weakest(b_stage, c_stage))),
                    (Some((b_index, stage)), None) => found.push((vec![Some(index), Some(b_index.clone()), None], stage.clone())),
                    (None, Some((c_index, stage))) => found.push((vec![Some(index), None, Some(c_index.clone())], stage.clone())),
                    (None, None) => {}
                }
            }
            let mut free_b = every(b);
            let mut free_c = every(c);
            for (entities, _) in found.iter() {
                if let Some(index) = entities[1] {
                    free_b[index] = false;
                }
                if let Some(index) = entities[2] {
                    free_c[index] = false;
                }
            }
            found.extend(pair(b, &free_b, c, &free_c, options).into_iter().map(|(b_index, c_index, stage)| (vec![None, Some(b_index), Some(c_index)], stage)));
            found
        }
        _ => Vec::new(),
    };
    if !options.missing_by_neighbors {
        found.retain(|(entities, _)| entities.iter().all(Option::is_some));
    }
    let complete: Vec<Vec<Range<usize>>> = found
        .iter()
        .filter(|(entities, _)| entities.iter().all(Option::is_some))
        .map(|(entities, _)| lines_of(documents, entities))
        .collect();
    let mut aligned: Vec<Aligned> = found
        .into_iter()
        .filter_map(|(entities, stage)| {
            let present = entities.iter().position(Option::is_some)?;
            let kind = documents[present].get(entities[present]?)?.kind.clone();
            let parts = match entities.iter().position(Option::is_none) {
                None => lines_of(documents, &entities),
                Some(missing) => {
                    let mut parts = lines_of(documents, &entities);
                    let anchor = anchor(&complete, present, &parts[present], missing)?;
                    parts[missing] = anchor..anchor;
                    parts
                }
            };
            let names = entities
                .iter()
                .zip(documents.iter())
                .map(|(index, entities)| index.and_then(|index| entities.get(index)).map(|entity| entity.name.clone()))
                .collect();
            Some(Aligned { kind, entities, parts, stage, names, parent: None })
        })
        .collect();
    aligned.sort_by_key(|aligned| aligned.parts.iter().map(|lines| (lines.start.clone(), lines.end.clone())).collect::<Vec<_>>());
    aligned
}


/// Per document, the lines of its entity, or an empty range at 0 where it has none.
fn lines_of(documents: &[&[Entity]], entities: &[Option<usize>]) -> Vec<Range<usize>> {
    entities
        .iter()
        .zip(documents.iter())
        .map(|(index, entities)| index.and_then(|index| entities.get(index)).map_or(0..0, |entity| entity.lines.clone()))
        .collect()
}


/// Where `missing` gets the anchor for an entity at `lines` of document `present`: before the complete alignment that follows it there, or after the one before it, when those two are neighbors in `missing` as well.
fn anchor(complete: &[Vec<Range<usize>>], present: usize, lines: &Range<usize>, missing: usize) -> Option<usize> {
    let previous = complete.iter().filter(|parts| parts[present].end <= lines.start).max_by_key(|parts| parts[present].end.clone());
    let next = complete.iter().filter(|parts| parts[present].start >= lines.end).min_by_key(|parts| parts[present].start.clone());
    let between = |after: usize, before: usize| complete.iter().any(|parts| parts[missing].start >= after && parts[missing].end <= before && !parts[missing].is_empty());
    match (previous, next) {
        (Some(previous), Some(next)) => {
            let (after, before) = (previous[missing].end.clone(), next[missing].start.clone());
            (after <= before && !between(after, before)).then_some(before)
        }
        (Some(previous), None) => {
            let after = previous[missing].end.clone();
            (!complete.iter().any(|parts| parts[missing].start >= after && !parts[missing].is_empty())).then_some(after)
        }
        (None, Some(next)) => {
            let before = next[missing].start.clone();
            (!complete.iter().any(|parts| parts[missing].end <= before && !parts[missing].is_empty())).then_some(before)
        }
        (None, None) => None,
    }
}


fn rank(stage: &MatchStage) -> u8 {
    match stage {
        MatchStage::Signature => 0,
        MatchStage::Name => 1,
        MatchStage::Similar { .. } => 2,
    }
}


fn weakest(first: &MatchStage, second: &MatchStage) -> MatchStage {
    match (first, second) {
        (MatchStage::Similar { signature: s1, body: b1 }, MatchStage::Similar { signature: s2, body: b2 }) => MatchStage::Similar {
            signature: s1.clone().min(s2.clone()),
            body: b1.clone().min(b2.clone()),
        },
        _ if rank(first) >= rank(second) => first.clone(),
        _ => second.clone(),
    }
}


/// Pairs the free entities of two documents: equal signatures first, then names that are unique among what is left, then similar signatures with similar bodies. Each stage pairs only what is unambiguous, and overloads pair by their signature alone unless `options` take the leftover one.
fn pair(a: &[Entity], free_a: &[bool], b: &[Entity], free_b: &[bool], options: &HeuristicOptions) -> Vec<(usize, usize, MatchStage)> {
    let mut free_a = free_a.to_vec();
    let mut free_b = free_b.to_vec();
    let mut pairs = Vec::new();
    let signatures_a = groups(a, &free_a, |entity| entity.signature.as_str());
    let signatures_b = groups(b, &free_b, |entity| entity.signature.as_str());
    for (key, first) in signatures_a.iter() {
        if let ([first], Some([second])) = (first.as_slice(), signatures_b.get(key).map(Vec::as_slice)) {
            pairs.push((first.clone(), second.clone(), MatchStage::Signature));
        }
    }
    take(&pairs, &mut free_a, &mut free_b);
    let names_a = groups(a, &vec![true; a.len()], |entity| entity.name.as_str());
    let names_b = groups(b, &vec![true; b.len()], |entity| entity.name.as_str());
    let overloaded = |groups: &HashMap<(EntityKind, &str), Vec<usize>>, entity: &Entity| entity.kind != EntityKind::Block && groups.get(&(entity.kind.clone(), entity.name.as_str())).is_some_and(|group| group.len() > 1);
    if options.match_names {
        let left_a = groups(a, &free_a, |entity| entity.name.as_str());
        let left_b = groups(b, &free_b, |entity| entity.name.as_str());
        let mut named = Vec::new();
        for (key, first) in left_a.iter() {
            if let ([first], Some([second])) = (first.as_slice(), left_b.get(key).map(Vec::as_slice)) {
                if options.overload_leftovers || (!overloaded(&names_a, &a[*first]) && !overloaded(&names_b, &b[*second])) {
                    named.push((first.clone(), second.clone(), MatchStage::Name));
                }
            }
        }
        take(&named, &mut free_a, &mut free_b);
        pairs.extend(named);
    }
    if options.match_similar {
        let eligible = |entity: &Entity, names: &HashMap<(EntityKind, &str), Vec<usize>>| options.overload_leftovers || !overloaded(names, entity);
        let bodies_a: Vec<HashMap<&str, usize>> = a.iter().map(body_counts).collect();
        let bodies_b: Vec<HashMap<&str, usize>> = b.iter().map(body_counts).collect();
        let mut candidates = Vec::new();
        for (first, entity_a) in a.iter().enumerate().filter(|(index, entity)| free_a[*index] && eligible(entity, &names_a)) {
            for (second, entity_b) in b.iter().enumerate().filter(|(index, entity)| free_b[*index] && entity.kind == entity_a.kind && eligible(entity, &names_b)) {
                let body = dice(&bodies_a[first], entity_a.body.len(), &bodies_b[second], entity_b.body.len());
                if body < options.body_similarity {
                    continue;
                }
                let signature = similarity(&entity_a.signature, &entity_b.signature, options.signature_similarity.clone());
                if signature < options.signature_similarity {
                    continue;
                }
                candidates.push((first, second, signature, body));
            }
        }
        let score = |candidate: &(usize, usize, u8, u8)| u32::from(candidate.2.clone()) + u32::from(candidate.3.clone());
        let best = |side: fn(&(usize, usize, u8, u8)) -> usize, index: usize| -> Option<usize> {
            let mut scores: Vec<(u32, usize)> = candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| side(candidate) == index)
                .map(|(position, candidate)| (score(candidate), position))
                .collect();
            scores.sort_by(|first, second| second.0.cmp(&first.0));
            match scores.as_slice() {
                [(top, position), (next, _), ..] if top > next => Some(position.clone()),
                [(_, position)] => Some(position.clone()),
                _ => None,
            }
        };
        let similar: Vec<(usize, usize, MatchStage)> = candidates
            .iter()
            .enumerate()
            .filter(|(position, candidate)| {
                best(|candidate| candidate.0, candidate.0) == Some(*position) && best(|candidate| candidate.1, candidate.1) == Some(*position)
            })
            .map(|(_, candidate)| {
                (
                    candidate.0.clone(),
                    candidate.1.clone(),
                    MatchStage::Similar {
                        signature: candidate.2.clone(),
                        body: candidate.3.clone(),
                    },
                )
            })
            .collect();
        pairs.extend(similar);
    }
    pairs.sort_by_key(|pair| pair.0.clone());
    pairs
}


/// The free entities of a document by kind and `key`, each group in document order.
fn groups<'a>(entities: &'a [Entity], free: &[bool], key: impl Fn(&'a Entity) -> &'a str) -> HashMap<(EntityKind, &'a str), Vec<usize>> {
    let mut groups: HashMap<(EntityKind, &str), Vec<usize>> = HashMap::new();
    for (index, entity) in entities.iter().enumerate().filter(|(index, _)| free[*index]) {
        groups.entry((entity.kind.clone(), key(entity))).or_default().push(index);
    }
    groups
}


fn take(pairs: &[(usize, usize, MatchStage)], free_a: &mut [bool], free_b: &mut [bool]) {
    for (first, second, _) in pairs.iter() {
        free_a[*first] = false;
        free_b[*second] = false;
    }
}


fn body_counts(entity: &Entity) -> HashMap<&str, usize> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in entity.body.iter() {
        *counts.entry(line.as_str()).or_default() += 1;
    }
    counts
}


/// The share of lines two bodies have in common, in percent: twice the common lines over all lines.
fn dice(first: &HashMap<&str, usize>, first_len: usize, second: &HashMap<&str, usize>, second_len: usize) -> u8 {
    if first_len + second_len == 0 {
        return 100;
    }
    let common: usize = first
        .iter()
        .map(|(line, count)| count.clone().min(second.get(line).cloned().unwrap_or(0)))
        .sum();
    u8::try_from(common * 200 / (first_len + second_len)).unwrap_or(100)
}


/// How alike two signatures are, in percent: 100 less the edit distance over the longer length. Signatures whose lengths alone keep them below `threshold` are not compared.
fn similarity(first: &str, second: &str, threshold: u8) -> u8 {
    let first: Vec<char> = first.chars().take(SIGNATURE_LIMIT).collect();
    let second: Vec<char> = second.chars().take(SIGNATURE_LIMIT).collect();
    let longest = first.len().max(second.len());
    if longest == 0 {
        return 100;
    }
    let shortest = first.len().min(second.len());
    if shortest * 100 / longest < usize::from(threshold) {
        return 0;
    }
    let distance = levenshtein(&first, &second);
    u8::try_from(100 - distance * 100 / longest).unwrap_or(0)
}


fn levenshtein(first: &[char], second: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=second.len()).collect();
    let mut current = vec![0; second.len() + 1];
    for (row, first_char) in first.iter().enumerate() {
        current[0] = row + 1;
        for (column, second_char) in second.iter().enumerate() {
            let substitution = previous[column] + usize::from(first_char != second_char);
            current[column + 1] = substitution.min(previous[column + 1] + 1).min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[second.len()]
}


#[cfg(test)]
mod tests {
    use super::*;

    fn entity(name: &str, signature: &str, lines: Range<usize>, body: &[&str]) -> Entity {
        Entity {
            kind: EntityKind::Function,
            name: name.to_string(),
            signature: signature.to_string(),
            header: lines.start.clone(),
            lines,
            body: body.iter().map(|line| line.to_string()).collect(),
            children: Vec::new(),
        }
    }

    fn block(header: &str, lines: Range<usize>, body: &[&str]) -> Entity {
        Entity {
            kind: EntityKind::Block,
            ..entity(header, header, lines, body)
        }
    }

    fn holding(mut entity: Entity, children: Vec<Entity>) -> Entity {
        entity.children = children;
        entity
    }

    fn tree(aligned: &[Aligned]) -> Vec<(Vec<Option<usize>>, Option<usize>)> {
        aligned.iter().map(|aligned| (aligned.entities.clone(), aligned.parent)).collect()
    }

    fn function(name: &str, lines: Range<usize>) -> Entity {
        entity(name, &format!("void {name}()"), lines, &[&format!("{name}_body();")])
    }

    fn entities(aligned: &[Aligned]) -> Vec<Vec<Option<usize>>> {
        aligned.iter().map(|aligned| aligned.entities.clone()).collect()
    }


    #[test]
    fn equal_signatures_pair_whatever_their_order() {
        let a = vec![function("f", 0..3), function("g", 4..7)];
        let b = vec![function("g", 0..3), function("f", 4..7)];
        let aligned = align(&[a, b], &HeuristicOptions::default());
        assert_eq!(entities(&aligned), vec![vec![Some(0), Some(1)], vec![Some(1), Some(0)]]);
        assert!(aligned.iter().all(|aligned| aligned.stage == MatchStage::Signature));
        assert_eq!(aligned[0].parts, vec![0..3, 4..7]);
    }


    #[test]
    fn a_changed_signature_pairs_by_its_unique_name() {
        let a = vec![entity("f", "void f(int)", 0..3, &["x();"])];
        let b = vec![entity("f", "void f(long, char)", 0..3, &["y();"])];
        let aligned = align(&[a.clone(), b.clone()], &HeuristicOptions::default());
        assert_eq!(aligned[0].stage, MatchStage::Name);
        let options = HeuristicOptions {
            match_names: false,
            ..HeuristicOptions::default()
        };
        assert!(align(&[a, b], &options).is_empty());
    }


    #[test]
    fn overloads_pair_only_by_exact_signature_unless_leftovers_are_allowed() {
        let a = vec![entity("f", "void f(int)", 0..3, &["x();"]), entity("f", "void f(double)", 4..7, &["y();"])];
        let b = vec![entity("f", "void f(int)", 0..3, &["x();"]), entity("f", "void f(float)", 4..7, &["y();"])];
        let aligned = align(&[a.clone(), b.clone()], &HeuristicOptions::default());
        assert_eq!(entities(&aligned), vec![vec![Some(0), Some(0)]]);
        let options = HeuristicOptions {
            overload_leftovers: true,
            ..HeuristicOptions::default()
        };
        assert_eq!(entities(&align(&[a, b], &options)), vec![vec![Some(0), Some(0)], vec![Some(1), Some(1)]]);
    }


    #[test]
    fn a_renamed_function_pairs_when_signature_and_body_are_similar() {
        let body = ["int total = 0;", "for (int x : values)", "total += x;", "return total;"];
        let a = vec![entity("sumValues", "int sumValues(const std::vector<int>&values)", 0..6, &body)];
        let b = vec![entity("sumAllValues", "int sumAllValues(const std::vector<int>&values)", 0..6, &body)];
        let aligned = align(&[a, b], &HeuristicOptions::default());
        assert!(matches!(aligned[0].stage, MatchStage::Similar { body: 100, .. }));
    }


    #[test]
    fn similar_names_with_different_bodies_do_not_pair() {
        let a = vec![entity("getX", "int getX()const", 0..3, &["return x;"]), entity("keep", "void keep()", 4..7, &["k();"])];
        let b = vec![entity("getY", "int getY()const", 0..3, &["return y;"]), entity("keep", "void keep()", 4..7, &["k();"])];
        assert_eq!(entities(&align(&[a, b], &HeuristicOptions::default())), vec![vec![Some(1), Some(1)]]);
    }


    #[test]
    fn an_ambiguous_similar_pair_is_left_out() {
        let body = ["work();"];
        let a = vec![entity("run1", "void run1()", 0..3, &body)];
        let b = vec![entity("run2", "void run2()", 0..3, &body), entity("run3", "void run3()", 4..7, &body)];
        assert!(align(&[a, b], &HeuristicOptions::default()).is_empty());
    }


    #[test]
    fn three_documents_pair_through_the_first() {
        let a = vec![function("f", 0..3), function("g", 4..7)];
        let b = vec![function("g", 0..3), function("f", 4..7)];
        let c = vec![function("f", 0..3), function("g", 4..7)];
        assert_eq!(entities(&align(&[a, b, c], &HeuristicOptions::default())), vec![vec![Some(0), Some(1), Some(0)], vec![Some(1), Some(0), Some(1)]]);
    }


    #[test]
    fn a_missing_entity_is_anchored_between_its_neighbors() {
        let a = vec![function("f", 0..3), function("g", 4..7), function("h", 8..11)];
        let b = vec![function("f", 0..3), function("g", 4..7), function("h", 8..11)];
        let c = vec![function("f", 0..3), function("h", 5..8)];
        let aligned = align(&[a.clone(), b.clone(), c.clone()], &HeuristicOptions::default());
        assert_eq!(aligned[1].entities, vec![Some(1), Some(1), None]);
        assert_eq!(aligned[1].parts, vec![4..7, 4..7, 5..5]);
        let options = HeuristicOptions {
            missing_by_neighbors: false,
            ..HeuristicOptions::default()
        };
        assert_eq!(align(&[a, b, c], &options).len(), 2);
    }


    #[test]
    fn a_missing_entity_at_the_end_is_anchored_after_the_last_one() {
        let a = vec![function("f", 0..3), function("g", 4..7)];
        let b = vec![function("f", 0..3)];
        let c = vec![function("f", 0..3), function("g", 3..6)];
        let aligned = align(&[a, b, c], &HeuristicOptions::default());
        assert_eq!(aligned[1].parts, vec![4..7, 3..3, 3..6]);
    }


    #[test]
    fn a_missing_entity_whose_neighbors_are_apart_is_skipped() {
        let a = vec![function("f", 0..3), function("g", 4..7), function("h", 8..11)];
        let b = vec![function("f", 0..3), function("g", 4..7), function("h", 8..11)];
        let c = vec![function("h", 0..3), function("f", 4..7)];
        assert_eq!(entities(&align(&[a, b, c], &HeuristicOptions::default())), vec![vec![Some(0), Some(0), Some(1)], vec![Some(2), Some(2), Some(0)]]);
    }


    #[test]
    fn an_entity_added_on_both_sides_pairs_b_with_c_and_anchors_in_a() {
        let a = vec![function("f", 0..3), function("h", 4..7)];
        let b = vec![function("f", 0..3), function("g", 4..7), function("h", 8..11)];
        let c = vec![function("f", 0..3), function("g", 4..7), function("h", 8..11)];
        let aligned = align(&[a, b, c], &HeuristicOptions::default());
        assert_eq!(aligned[1].entities, vec![None, Some(1), Some(1)]);
        assert_eq!(aligned[1].parts, vec![4..4, 4..7, 4..7]);
    }


    #[test]
    fn similarity_and_dice_are_percentages() {
        assert_eq!(similarity("abcd", "abcd", 0), 100);
        assert_eq!(similarity("abcd", "abce", 0), 75);
        assert_eq!(similarity("a", "abcdefghij", 80), 0);
        assert_eq!(levenshtein(&['k', 'i', 't'], &['s', 'i', 't', 's']), 2);
        let (first, second) = (entity("f", "", 0..1, &["a", "b"]), entity("f", "", 0..1, &["a", "c"]));
        assert_eq!(dice(&body_counts(&first), 2, &body_counts(&second), 2), 50);
    }


    fn blocks_of(first: &str, second: &str, third: &str) -> Vec<Entity> {
        vec![block(first, 2..5, &["a();"]), block(second, 6..9, &["b();"]), block(third, 10..13, &["c();"])]
    }


    #[test]
    fn the_children_of_paired_entities_are_paired_after_their_parent() {
        let a = vec![holding(function("f", 0..14), blocks_of("if(x)", "for(;;)", "while(y)")), function("g", 15..18)];
        let b = vec![function("g", 0..3), holding(function("f", 4..18), blocks_of("while(y)", "if(x)", "for(;;)").into_iter().map(|mut block| { block.lines = block.lines.start + 4..block.lines.end + 4; block }).collect())];
        let aligned = align(&[a, b], &HeuristicOptions::default());
        assert_eq!(aligned.len(), 4);
        assert_eq!(aligned[0].names, vec![Some("f".to_string()), Some("f".to_string())]);
        assert_eq!(aligned[0].parent, None);
        assert_eq!(aligned[0].parts, vec![0..14, 4..18]);
        assert!(aligned[1..3].iter().all(|aligned| aligned.kind == EntityKind::Block && aligned.parent == Some(0)));
        assert_eq!(aligned[3].names, vec![Some("g".to_string()), Some("g".to_string())]);
        assert_eq!(aligned[3].parent, None);
    }


    #[test]
    fn blocks_that_changed_their_order_are_dropped_but_members_of_a_type_may_be_reordered() {
        let shifted = |blocks: Vec<Entity>| -> Vec<Entity> { blocks.into_iter().map(|mut block| { block.lines = block.lines.start + 1..block.lines.end + 1; block }).collect() };
        let a = vec![holding(function("f", 0..14), blocks_of("if(x)", "for(;;)", "while(y)"))];
        let b = vec![holding(function("f", 0..14), shifted(blocks_of("while(y)", "for(;;)", "if(x)")))];
        let aligned = align(&[a, b], &HeuristicOptions::default());
        assert_eq!(aligned.len(), 2);
        assert_eq!(aligned[1].names, vec![Some("if(x)".to_string()), Some("if(x)".to_string())]);
        assert_eq!(aligned[1].parent, Some(0));
        let type_of = |first: &str, second: &str| Entity {
            kind: EntityKind::Type,
            ..holding(entity("T", "classT", 0..14, &[]), vec![function(first, 2..5), function(second, 6..9)])
        };
        let aligned = align(&[vec![type_of("m", "n")], vec![type_of("n", "m")]], &HeuristicOptions::default());
        assert_eq!(tree(&aligned), vec![(vec![Some(0), Some(0)], None), (vec![Some(0), Some(1)], Some(0)), (vec![Some(1), Some(0)], Some(0))]);
    }


    #[test]
    fn a_child_one_document_lacks_is_anchored_among_its_siblings() {
        let a = vec![holding(function("f", 0..14), blocks_of("if(x)", "for(;;)", "while(y)"))];
        let b = vec![holding(function("f", 0..14), blocks_of("if(x)", "for(;;)", "while(y)"))];
        let c = vec![holding(function("f", 0..10), vec![block("if(x)", 2..5, &["a();"]), block("while(y)", 6..9, &["c();"])])];
        let aligned = align(&[a, b, c], &HeuristicOptions::default());
        assert_eq!(aligned.len(), 4);
        assert_eq!(aligned[2].entities, vec![Some(1), Some(1), None]);
        assert_eq!(aligned[2].parts, vec![6..9, 6..9, 6..6]);
        assert_eq!(aligned[2].parent, Some(0));
    }
}
