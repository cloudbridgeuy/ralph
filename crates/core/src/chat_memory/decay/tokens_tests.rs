use super::tokens::{classify, document_frequency, key_tokens, Class};
use crate::chat_memory::fixtures::texted;
use crate::chat_memory::search::tokenize;

fn class_of(word: &str) -> Option<Class> {
    let frequency = document_frequency(&texted(1, |_| word.to_string()));
    classify(&tokenize(word)[0], &frequency)
}

#[test]
fn classify_ticket_number_identifier() {
    assert_eq!(class_of("guz-1637"), Some(Class::Ticket));
    assert_eq!(class_of("4096"), Some(Class::Number));
    assert_eq!(class_of("split_view"), Some(Class::Identifier));
    assert_eq!(class_of("chat::view"), Some(Class::Identifier));
    assert_eq!(class_of("a-b"), Some(Class::Identifier));
    assert_eq!(class_of("it"), None);
}

#[test]
fn rare_term_df_at_most_three() {
    let entries = texted(6, |i| {
        if i < 3 {
            "pelican common".to_string()
        } else {
            "common walrus".to_string()
        }
    });
    let frequency = document_frequency(&entries);
    let tokens = tokenize("pelican common walrus");
    assert_eq!(classify(&tokens[0], &frequency), Some(Class::Rare));
    assert_eq!(classify(&tokens[1], &frequency), None);
    assert_eq!(classify(&tokens[2], &frequency), Some(Class::Rare));
    let crowded = texted(4, |_| "pelican".to_string());
    let frequency = document_frequency(&crowded);
    assert_eq!(classify(&tokens[0], &frequency), None);
}

#[test]
fn short_plain_words_are_ignored_even_when_rare() {
    assert_eq!(class_of("cat"), None);
}

#[test]
fn tokens_use_search_tokenizer_parts() {
    let frequency = document_frequency(&[]);
    let keys = key_tokens("a-b x::y", &frequency);
    let seen: Vec<(&str, Class)> = keys.iter().map(|k| (k.word.as_str(), k.class)).collect();
    assert_eq!(
        seen,
        [("a-b", Class::Identifier), ("x::y", Class::Identifier)]
    );
}

#[test]
fn key_tokens_dedupe_and_keep_order() {
    let frequency = document_frequency(&[]);
    let keys = key_tokens("GUZ-1637 up guz-1637 and 42", &frequency);
    let seen: Vec<(&str, Class)> = keys.iter().map(|k| (k.word.as_str(), k.class)).collect();
    assert_eq!(seen, [("guz-1637", Class::Ticket), ("42", Class::Number)]);
}
