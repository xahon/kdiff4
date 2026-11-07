use std::ops::Range;

use kdiff4_protocol::{EntityKind, Extractor};
use tree_sitter::{Language, Node, Parser, Tree};

const TYPE_KINDS: &[&str] = &["class_specifier", "struct_specifier", "union_specifier", "enum_specifier"];
/// Nodes that wrap an entity without adding anything but its own lines: a template header, or the declaration or typedef of a type.
/// The statements that are blocks inside a function.
const BLOCK_KINDS: &[&str] = &["if_statement", "for_statement", "for_range_loop", "while_statement", "do_statement", "switch_statement", "case_statement", "try_statement"];
/// A block spans at least this many lines, from its header to its closing line; smaller ones are not aligned.
const MIN_BLOCK_LINES: usize = 3;
const WRAPPERS: &[&str] = &["template_declaration", "declaration", "type_definition", "field_declaration"];


/// A function, method, type or block found in one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub kind: EntityKind,
    /// Qualified with the namespaces and types around it; a block is named by its header.
    pub name: String,
    /// The text before the body without comments, with whitespace only between two words.
    pub signature: String,
    /// From the comments right above it to the line of its closing brace.
    pub lines: Range<usize>,
    /// The trimmed lines of its body that hold a letter or a digit.
    pub body: Vec<String>,
    /// The members of a type and the blocks of a function or block: ordered by position, not overlapping and inside `lines`.
    pub children: Vec<Entity>,
}


/// Whether `extractor` can parse `language`, a name the language selector shows.
pub fn supports(extractor: Extractor, language: &str) -> bool {
    match extractor {
        Extractor::TreeSitter => !grammars(language).is_empty(),
    }
}


/// The entities of `kinds` in one document, ordered by position and not overlapping. `None` when `extractor` can't parse `language`.
pub fn extract(extractor: Extractor, language: &str, text: &str, lines: &[Range<usize>], kinds: &[EntityKind]) -> Option<Vec<Entity>> {
    match extractor {
        Extractor::TreeSitter => extract_tree_sitter(language, text, lines, kinds),
    }
}


/// The grammars to try for `language`, best first. A C header may hold C++, so C falls back to the C++ grammar.
fn grammars(language: &str) -> Vec<Language> {
    match language {
        "C" => vec![tree_sitter_c::LANGUAGE.into(), tree_sitter_cpp::LANGUAGE.into()],
        "C++" | "CUDA" => vec![tree_sitter_cpp::LANGUAGE.into()],
        _ => Vec::new(),
    }
}


fn extract_tree_sitter(language: &str, text: &str, lines: &[Range<usize>], kinds: &[EntityKind]) -> Option<Vec<Entity>> {
    let grammars = grammars(language);
    if grammars.is_empty() {
        return None;
    }
    let mut best: Option<Tree> = None;
    for grammar in grammars.iter() {
        let mut parser = Parser::new();
        if parser.set_language(grammar).is_err() {
            log::warn!("heuristics: the tree-sitter grammar for {language} does not fit the tree-sitter library");
            continue;
        }
        let Some(tree) = parser.parse(text, None) else {
            continue;
        };
        let clean = !tree.root_node().has_error();
        if best.as_ref().is_none_or(|best| error_count(tree.root_node()) < error_count(best.root_node())) {
            best = Some(tree);
        }
        if clean {
            break;
        }
    }
    let Some(tree) = best else {
        return Some(Vec::new());
    };
    let walker = Walker {
        text,
        lines,
        kinds,
    };
    Some(tidy(walker.visit_children(tree.root_node(), "", false)))
}


fn error_count(node: Node) -> usize {
    if !node.has_error() {
        return 0;
    }
    let mut cursor = node.walk();
    let own = usize::from(node.is_error() || node.is_missing());
    own + node.children(&mut cursor).map(error_count).sum::<usize>()
}


struct Walker<'a> {
    text: &'a str,
    lines: &'a [Range<usize>],
    kinds: &'a [EntityKind],
}

impl Walker<'_> {
    fn visit_children(&self, node: Node, scope: &str, in_type: bool) -> Vec<Entity> {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        children.into_iter().flat_map(|child| self.visit(child, scope, in_type)).collect()
    }


    fn visit(&self, node: Node, scope: &str, in_type: bool) -> Vec<Entity> {
        match node.kind() {
            "function_definition" => self.function(node, scope, in_type),
            kind if TYPE_KINDS.contains(&kind) => self.type_entity(node, scope),
            "namespace_definition" => {
                let name = node.child_by_field_name("name").map(|name| self.normalized(name.start_byte(), name.end_byte(), &[])).unwrap_or_default();
                match node.child_by_field_name("body") {
                    Some(body) => self.visit_children(body, &qualify(scope, &name), false),
                    None => Vec::new(),
                }
            }
            "compound_statement" | "comment" | "string_literal" | "raw_string_literal" => Vec::new(),
            _ => self.visit_children(node, scope, in_type),
        }
    }


    fn function(&self, node: Node, scope: &str, in_type: bool) -> Vec<Entity> {
        let Some(body) = node.child_by_field_name("body") else {
            return Vec::new();
        };
        let Some(name) = function_name(node) else {
            return Vec::new();
        };
        let name = self.normalized(name.start_byte(), name.end_byte(), &[]);
        let kind = if in_type || name.contains("::") { EntityKind::Method } else { EntityKind::Function };
        if !self.kinds.contains(&kind) {
            return Vec::new();
        }
        let outer = outermost(node);
        let mut cursor = node.walk();
        let initializers = node.children(&mut cursor).find(|child| child.kind() == "field_initializer_list");
        let end = initializers.map_or(body.start_byte(), |initializers| initializers.start_byte());
        let qualified = qualify(scope, &name);
        let children = self.blocks_in(body, &qualified);
        vec![self.entity(kind, Some(qualified), outer, end, body.start_byte(), children)]
    }


    /// The type itself when `types` is on, holding the entities of its members; otherwise the entities of its members. Enums count as a whole only.
    fn type_entity(&self, node: Node, scope: &str) -> Vec<Entity> {
        let Some(body) = node.child_by_field_name("body") else {
            return Vec::new();
        };
        let outer = outermost(node);
        let name = node
            .child_by_field_name("name")
            .or_else(|| (outer.kind() == "type_definition").then(|| outer.child_by_field_name("declarator")).flatten())
            .map(|name| self.normalized(name.start_byte(), name.end_byte(), &[]));
        let qualified = qualify(scope, name.as_deref().unwrap_or(""));
        let members = if node.kind() == "enum_specifier" { Vec::new() } else { self.visit_children(body, &qualified, true) };
        if name.is_none() || !self.kinds.contains(&EntityKind::Type) {
            return members;
        }
        vec![self.entity(EntityKind::Type, Some(qualified), outer, body.start_byte(), body.start_byte(), members)]
    }


    /// The blocks, nested functions and local types inside `node`, which is the body of a function, a block or a lambda. Nothing when blocks are off.
    fn blocks_in(&self, node: Node, scope: &str) -> Vec<Entity> {
        if !self.kinds.contains(&EntityKind::Block) {
            return Vec::new();
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        children.into_iter().flat_map(|child| self.block_or_inside(child, scope)).collect()
    }


    fn block_or_inside(&self, node: Node, scope: &str) -> Vec<Entity> {
        match node.kind() {
            "comment" | "string_literal" | "raw_string_literal" | "number_literal" | "char_literal" => Vec::new(),
            "function_definition" => self.nested_function(node, scope),
            "lambda_expression" => self.lambda(node, scope),
            kind if TYPE_KINDS.contains(&kind) => self.type_entity(node, scope),
            kind if BLOCK_KINDS.contains(&kind) => self.block(node, scope),
            _ => self.blocks_in(node, scope),
        }
    }


    fn block(&self, node: Node, scope: &str) -> Vec<Entity> {
        let Some((header_end, body_start)) = block_header(node) else {
            return self.blocks_in(node, scope);
        };
        let children = match node.kind() {
            "if_statement" => self.if_children(node, scope),
            _ => self.blocks_in(node, scope),
        };
        let Some(mut entity) = self.block_entity(node, header_end, body_start, children.clone()) else {
            return children;
        };
        if node.kind() == "do_statement" {
            let condition = node.child_by_field_name("condition").map(|condition| self.normalized(condition.start_byte(), condition.end_byte(), &[])).unwrap_or_default();
            entity.signature = format!("do while {condition}");
            entity.name = entity.signature.clone();
        }
        vec![entity]
    }


    /// The blocks in the branches of an `if` and of the `else if` and `else` after it, which are not blocks of their own.
    fn if_children(&self, node: Node, scope: &str) -> Vec<Entity> {
        let mut children = Vec::new();
        if let Some(consequence) = node.child_by_field_name("consequence") {
            children.extend(self.block_or_inside(consequence, scope));
        }
        let Some(alternative) = node.child_by_field_name("alternative") else {
            return children;
        };
        let mut cursor = alternative.walk();
        let branches: Vec<Node> = if alternative.kind() == "else_clause" { alternative.named_children(&mut cursor).collect() } else { vec![alternative] };
        for branch in branches {
            if branch.kind() == "if_statement" {
                children.extend(self.if_children(branch, scope));
            } else {
                children.extend(self.block_or_inside(branch, scope));
            }
        }
        children
    }


    fn nested_function(&self, node: Node, scope: &str) -> Vec<Entity> {
        let Some(body) = node.child_by_field_name("body") else {
            return Vec::new();
        };
        let children = self.blocks_in(body, scope);
        match self.block_entity(node, body.start_byte(), body.start_byte(), children.clone()) {
            Some(entity) => vec![entity],
            None => children,
        }
    }


    /// A lambda is a block as the statement that holds it, when that statement starts its line and the lambda is not in the header of another block.
    fn lambda(&self, node: Node, scope: &str) -> Vec<Entity> {
        let Some(body) = node.child_by_field_name("body") else {
            return Vec::new();
        };
        let inside = self.blocks_in(body, scope);
        let mut statement = node;
        loop {
            let Some(parent) = statement.parent() else {
                return inside;
            };
            if is_statement_of(parent, statement) {
                break;
            }
            if BLOCK_KINDS.contains(&parent.kind()) {
                return inside;
            }
            statement = parent;
        }
        if !self.starts_line(statement) {
            return inside;
        }
        match self.block_entity(statement, body.start_byte(), body.start_byte(), inside.clone()) {
            Some(entity) => vec![entity],
            None => inside,
        }
    }


    /// A block made of `node` with its template header, or `None` when it spans fewer than `MIN_BLOCK_LINES` lines.
    fn block_entity(&self, node: Node, header_end: usize, body_start: usize, children: Vec<Entity>) -> Option<Entity> {
        let entity = self.entity(EntityKind::Block, None, outermost(node), header_end, body_start, children);
        (entity.lines.len() >= MIN_BLOCK_LINES).then_some(entity)
    }


    /// An entity of `outer`, named `name` or, without one, by its signature, the text of `outer` up to `signature_end`.
    fn entity(&self, kind: EntityKind, name: Option<String>, outer: Node, signature_end: usize, body_start: usize, children: Vec<Entity>) -> Entity {
        let comments = comments_before(outer, signature_end);
        let signature = self.normalized(outer.start_byte(), signature_end, &comments);
        let start = self.first_line(outer);
        let end = self.line_of(outer.end_byte().saturating_sub(1)) + 1;
        let body_start = self.line_of(body_start);
        let body = (body_start..end)
            .filter_map(|line| self.lines.get(line).and_then(|range| self.text.get(range.clone())))
            .map(str::trim)
            .filter(|line| line.chars().any(char::is_alphanumeric))
            .map(str::to_string)
            .collect();
        Entity {
            kind,
            name: name.unwrap_or_else(|| signature.clone()),
            signature,
            lines: start..end,
            body,
            children: tidy(children),
        }
    }


    /// The line `outer` starts on, moved up over the comments right above it: each must start its line and end on the line before the next.
    fn first_line(&self, outer: Node) -> usize {
        let mut first = self.line_of(outer.start_byte());
        let mut current = outer;
        while let Some(previous) = current.prev_named_sibling() {
            if previous.kind() != "comment" || self.line_of(previous.end_byte().saturating_sub(1)) + 1 != first || !self.starts_line(previous) {
                break;
            }
            first = self.line_of(previous.start_byte());
            current = previous;
        }
        first
    }


    fn starts_line(&self, node: Node) -> bool {
        let line = self.line_of(node.start_byte());
        let start = self.lines.get(line).map_or(0, |range| range.start.clone());
        self.text.get(start..node.start_byte()).is_some_and(|before| before.trim().is_empty())
    }


    fn line_of(&self, byte: usize) -> usize {
        self.lines.partition_point(|range| range.start <= byte).saturating_sub(1)
    }


    /// The text of `start..end` without the byte ranges `skipped`, with whitespace kept only as one space between two word characters.
    fn normalized(&self, start: usize, end: usize, skipped: &[Range<usize>]) -> String {
        let mut result = String::new();
        let mut gap = false;
        for (offset, character) in self.text.get(start..end).unwrap_or("").char_indices() {
            let position = start + offset;
            if character.is_whitespace() || skipped.iter().any(|range| range.contains(&position)) {
                gap = true;
                continue;
            }
            if gap && result.chars().last().is_some_and(is_word) && is_word(character) {
                result.push(' ');
            }
            gap = false;
            result.push(character);
        }
        result
    }
}


/// Orders `entities` by position and drops the empty ones and those that overlap the one before.
fn tidy(mut entities: Vec<Entity>) -> Vec<Entity> {
    entities.sort_by_key(|entity| (entity.lines.start.clone(), entity.lines.end.clone()));
    let mut kept: Vec<Entity> = Vec::with_capacity(entities.len());
    for entity in entities {
        if entity.lines.is_empty() || kept.last().is_some_and(|last| last.lines.end > entity.lines.start) {
            continue;
        }
        kept.push(entity);
    }
    kept
}


/// Where the header of a block ends and its body starts, in bytes.
fn block_header(node: Node) -> Option<(usize, usize)> {
    match node.kind() {
        "if_statement" => node.child_by_field_name("consequence").map(|body| (body.start_byte(), body.start_byte())),
        "case_statement" => {
            let mut cursor = node.walk();
            let colon = node.children(&mut cursor).find(|child| child.kind() == ":")?;
            Some((colon.start_byte(), colon.end_byte()))
        }
        _ => node.child_by_field_name("body").map(|body| (body.start_byte(), body.start_byte())),
    }
}


/// Whether `child` is a statement of `parent` rather than a part of its header.
fn is_statement_of(parent: Node, child: Node) -> bool {
    let is_field = |name: &str| parent.child_by_field_name(name).is_some_and(|field| field.id() == child.id());
    match parent.kind() {
        "compound_statement" | "else_clause" => true,
        "case_statement" => !is_field("value"),
        _ => is_field("body") || is_field("consequence"),
    }
}


fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}


fn qualify(scope: &str, name: &str) -> String {
    match (scope.is_empty(), name.is_empty()) {
        (true, _) => name.to_string(),
        (false, true) => scope.to_string(),
        (false, false) => format!("{scope}::{name}"),
    }
}


/// The node that holds `node` with its template header or declaration.
fn outermost(node: Node) -> Node {
    let mut outer = node;
    while let Some(parent) = outer.parent() {
        if !WRAPPERS.contains(&parent.kind()) {
            break;
        }
        outer = parent;
    }
    outer
}


/// The name in the function declarator of a function definition, under any pointer or reference declarators.
fn function_name(node: Node) -> Option<Node> {
    let mut declarator = node.child_by_field_name("declarator")?;
    while declarator.kind() != "function_declarator" {
        declarator = declarator.child_by_field_name("declarator").or_else(|| declarator.named_child(0))?;
    }
    declarator.child_by_field_name("declarator")
}


/// The byte ranges of the comments in `node` that start before `end`.
fn comments_before(node: Node, end: usize) -> Vec<Range<usize>> {
    let mut comments = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() >= end {
            break;
        }
        if child.kind() == "comment" {
            comments.push(child.start_byte()..child.end_byte());
        } else if child.child_count() > 0 {
            comments.extend(comments_before(child, end));
        }
    }
    comments
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::split_lines;

    const ALL: &[EntityKind] = &[EntityKind::Function, EntityKind::Method, EntityKind::Type];
    const WITHOUT_TYPES: &[EntityKind] = &[EntityKind::Function, EntityKind::Method];
    const WITH_BLOCKS: &[EntityKind] = &[EntityKind::Function, EntityKind::Method, EntityKind::Block];

    fn entities(language: &str, text: &str, kinds: &[EntityKind]) -> Vec<(EntityKind, String, Range<usize>)> {
        let lines = split_lines(text);
        extract(Extractor::TreeSitter, language, text, &lines, kinds)
            .expect("Test: the language is supported")
            .into_iter()
            .map(|entity| (entity.kind, entity.name, entity.lines))
            .collect()
    }

    /// The entities with their children, one per line: kind, name and lines, indented by depth.
    fn tree(language: &str, text: &str, kinds: &[EntityKind]) -> Vec<String> {
        fn render(entities: &[Entity], depth: usize, output: &mut Vec<String>) {
            for entity in entities {
                output.push(format!("{}{:?} {} {}..{}", "  ".repeat(depth), entity.kind, entity.name, entity.lines.start, entity.lines.end));
                render(&entity.children, depth + 1, output);
            }
        }
        let lines = split_lines(text);
        let mut output = Vec::new();
        render(&extract(Extractor::TreeSitter, language, text, &lines, kinds).expect("Test: the language is supported"), 0, &mut output);
        output
    }

    fn signatures(language: &str, text: &str) -> Vec<String> {
        let lines = split_lines(text);
        extract(Extractor::TreeSitter, language, text, &lines, ALL)
            .expect("Test: the language is supported")
            .into_iter()
            .map(|entity| entity.signature)
            .collect()
    }


    #[test]
    fn only_c_family_languages_are_supported() {
        assert!(supports(Extractor::TreeSitter, "C++"));
        assert!(supports(Extractor::TreeSitter, "C"));
        assert!(supports(Extractor::TreeSitter, "CUDA"));
        assert!(!supports(Extractor::TreeSitter, "Plain text"));
        assert_eq!(extract(Extractor::TreeSitter, "Python", "def f():\n    pass\n", &split_lines("def f():\n    pass\n"), ALL), None);
    }


    #[test]
    fn functions_take_namespaces_and_the_comments_right_above_them() {
        let text = "namespace config {\n\n// Trims.\n// Both ends.\nstd::string trim(const std::string &text)\n{\n    return text;\n}\n\n/// Not attached.\n\nint f() { return 0; }\n}\n";
        assert_eq!(
            entities("C++", text, ALL),
            vec![(EntityKind::Function, "config::trim".to_string(), 2..8), (EntityKind::Function, "config::f".to_string(), 11..12)]
        );
    }


    #[test]
    fn a_comment_after_code_on_its_line_stays_with_that_code() {
        let text = "int a() { return 1; } // a\nint b() { return 2; }\n";
        assert_eq!(entities("C++", text, ALL), vec![(EntityKind::Function, "a".to_string(), 0..1), (EntityKind::Function, "b".to_string(), 1..2)]);
    }


    #[test]
    fn templates_qualified_names_and_class_members_are_methods() {
        let text = "template<class T>\nT &Box<T>::get() const\n{\n    return value;\n}\nstruct S {\n    void m()\n    {\n    }\n    S() : x(1) {}\n    int x;\n};\n";
        assert_eq!(
            entities("C++", text, WITHOUT_TYPES),
            vec![
                (EntityKind::Method, "Box<T>::get".to_string(), 0..5),
                (EntityKind::Method, "S::m".to_string(), 6..9),
                (EntityKind::Method, "S::S".to_string(), 9..10)
            ]
        );
    }


    #[test]
    fn a_type_holds_its_members_when_types_are_on_and_leaves_them_at_its_level_otherwise() {
        let text = "struct S {
    int x;
    void m() {}
};
enum class E {
    A,
};
struct P {
    int y;
};
";
        assert_eq!(entities("C++", text, WITHOUT_TYPES), vec![(EntityKind::Method, "S::m".to_string(), 2..3)]);
        assert_eq!(
            entities("C++", text, ALL),
            vec![(EntityKind::Type, "S".to_string(), 0..4), (EntityKind::Type, "E".to_string(), 4..7), (EntityKind::Type, "P".to_string(), 7..10)]
        );
        assert_eq!(tree("C++", text, ALL), ["Type S 0..4", "  Method S::m 2..3", "Type E 4..7", "Type P 7..10"]);
        assert_eq!(tree("C++", text, &[EntityKind::Function, EntityKind::Type]), ["Type S 0..4", "Type E 4..7", "Type P 7..10"]);
    }


    #[test]
    fn kinds_that_are_off_are_not_extracted() {
        let text = "void f() {}\nstruct S { void m() {} };\nenum E { A };\n";
        assert_eq!(entities("C++", text, &[EntityKind::Function]), vec![(EntityKind::Function, "f".to_string(), 0..1)]);
    }


    #[test]
    fn c_typedefs_name_anonymous_structs_and_declarations_are_skipped() {
        let text = "typedef struct {\n    int a;\n} T;\nstatic int *g(void);\nstatic int *g(void)\n{\n    return 0;\n}\n";
        assert_eq!(entities("C", text, ALL), vec![(EntityKind::Type, "T".to_string(), 0..3), (EntityKind::Function, "g".to_string(), 4..8)]);
    }


    #[test]
    fn a_c_header_with_classes_falls_back_to_cpp() {
        let text = "class Widget {\npublic:\n    int size() const { return 1; }\n};\n";
        assert_eq!(entities("C", text, WITHOUT_TYPES), vec![(EntityKind::Method, "Widget::size".to_string(), 2..3)]);
    }


    #[test]
    fn signatures_drop_comments_initializers_and_formatting() {
        let text = "std::string  &  join(const std::vector<std::string>& parts, /* sep */ char c)\n{\n}\nS::S(int a) : x(a)\n{\n}\n";
        assert_eq!(signatures("C++", text), vec!["std::string&join(const std::vector<std::string>&parts,char c)".to_string(), "S::S(int a)".to_string()]);
    }


    #[test]
    fn bodies_keep_lines_with_words() {
        let text = "int f()\n{\n    int x = 1;\n\n    return x;\n}\n";
        let lines = split_lines(text);
        let entity = extract(Extractor::TreeSitter, "C++", text, &lines, ALL).expect("Test: the language is supported").remove(0);
        assert_eq!(entity.body, vec!["int x = 1;".to_string(), "return x;".to_string()]);
    }


    #[test]
    fn crlf_lines_map_to_the_right_lines() {
        let text = "int a()\r\n{\r\n}\r\n\r\nint b()\r\n{\r\n}\r\n";
        assert_eq!(entities("C++", text, ALL), vec![(EntityKind::Function, "a".to_string(), 0..3), (EntityKind::Function, "b".to_string(), 4..7)]);
    }


    const BODY: &str = "void f(int n)
{
    // Prepare.
    if (n > 0)
    {
        run();
    }
    else if (n < 0)
    {
        undo();
    }
    else
    {
        idle();
    }
    for (int i = 0; i < n; ++i)
    {
        step(i);
    }
    while (n--)
    {
        wait();
    }
    do
    {
        again();
    } while (n > 1);
    switch (n)
    {
        case 1:
            one();
            break;
        default:
            other();
            break;
    }
    try
    {
        risky();
    }
    catch (...)
    {
        recover();
    }
    auto done = [&](int x)
    {
        return x + n;
    };
    if (n) return;
}
";

    #[test]
    fn blocks_are_found_inside_functions() {
        assert_eq!(
            tree("C++", BODY, WITH_BLOCKS),
            [
                "Function f 0..50",
                "  Block if(n>0) 2..15",
                "  Block for(int i=0;i<n;++i) 15..19",
                "  Block while(n--) 19..23",
                "  Block do while (n>1) 23..27",
                "  Block switch(n) 27..36",
                "    Block case 1 29..32",
                "    Block default 32..35",
                "  Block try 36..44",
                "  Block auto done=[&](int x) 44..48"
            ]
        );
    }


    #[test]
    fn blocks_are_off_without_the_block_kind_and_small_ones_are_skipped() {
        assert_eq!(tree("C++", BODY, WITHOUT_TYPES), ["Function f 0..50"]);
        let small = "void f()
{
    if (a)
        b();
    for (;;) { c(); }
    while (d)
    {
        e();
    }
}
";
        assert_eq!(tree("C++", small, WITH_BLOCKS), ["Function f 0..10", "  Block while(d) 5..9"]);
    }


    #[test]
    fn an_if_chain_is_one_block_and_its_branches_hold_the_blocks_inside() {
        let text = "void f(int n)
{
    if (n > 0)
    {
        for (;;)
        {
            a();
        }
    }
    else
    {
        while (n)
        {
            b();
        }
    }
}
";
        assert_eq!(
            tree("C++", text, WITH_BLOCKS),
            ["Function f 0..17", "  Block if(n>0) 2..16", "    Block for(;;) 4..8", "    Block while(n) 11..15"]
        );
    }


    #[test]
    fn a_lambda_in_a_call_is_a_block_only_when_its_statement_starts_the_line() {
        let text = "void f()
{
    run(1,
        [](int x)
        {
            return x;
        });
    int y = call([&]
    {
        return 1;
    });
}
";
        assert_eq!(tree("C++", text, WITH_BLOCKS), ["Function f 0..12", "  Block run(1,[](int x) 2..7", "  Block int y=call([&] 7..11"]);
        let mid = "void g()
{
    int a = 1; auto h = []
    {
        return 2;
    };
}
";
        assert_eq!(tree("C++", mid, WITH_BLOCKS), ["Function g 0..7"]);
    }


    #[test]
    fn local_types_and_their_methods_are_inside_the_function() {
        let text = "void f()
{
    struct Local
    {
        int get()
        {
            return 1;
        }
    };
}
";
        assert_eq!(tree("C++", text, ALL.iter().cloned().chain([EntityKind::Block]).collect::<Vec<_>>().as_slice()), ["Function f 0..10", "  Type f::Local 2..9", "    Method f::Local::get 4..8"]);
        assert_eq!(tree("C++", text, WITH_BLOCKS), ["Function f 0..10", "  Method f::Local::get 4..8"]);
    }
}
