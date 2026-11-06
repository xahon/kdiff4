use std::collections::HashMap;

use kdiff4_protocol::engine::Token;

/// Interns the line keys of all documents together, so equal keys get equal tokens in every document. A line's key is its text until ignore options arrive with the engine config.
pub fn tokenize(documents: &[Vec<&str>]) -> Vec<Vec<Token>> {
    let mut interned: HashMap<&str, Token> = HashMap::new();
    let mut tokens = Vec::with_capacity(documents.len());
    for lines in documents {
        let mut document_tokens = Vec::with_capacity(lines.len());
        for line in lines {
            let next = Token(interned.len() as u32);
            document_tokens.push(interned.entry(line).or_insert(next).clone());
        }
        tokens.push(document_tokens);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_lines_share_tokens_across_documents() {
        let tokens = tokenize(&[vec!["a", "b", "a"], vec!["b", "c"]]);
        assert_eq!(tokens, [vec![Token(0), Token(1), Token(0)], vec![Token(1), Token(2)]]);
    }

    #[test]
    fn lines_differing_only_in_whitespace_or_case_differ() {
        let tokens = tokenize(&[vec!["a", "a ", "A"]]);
        assert_eq!(tokens, [vec![Token(0), Token(1), Token(2)]]);
    }

    #[test]
    fn empty_documents() {
        assert_eq!(tokenize(&[vec![], vec![]]), [Vec::<Token>::new(), Vec::new()]);
    }
}
